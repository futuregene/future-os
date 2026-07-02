//! Core bridge logic: Feishu events → Agent → Feishu responses.
//!
//! Orchestrates the flow: receive Feishu event → check permissions →
//! handle slash commands locally → prompt agent via gRPC →
//! stream response back to Feishu via cards.

use super::card;
use super::config::FeishuConfig;
use super::feishu_rest::{bytes_to_base64_data, mime_from_ext, FeishuRestClient};
use super::feishu_ws::{
    extract_file_key, extract_image_key, extract_text_content, is_bot_mentioned,
    is_bot_mentioned_in_mentions, FeishuEvent,
};
use super::policy::{Access, PolicyEngine};
use super::session_store::SessionStore;
use crate::config::AgentConfig;
use crate::grpc_client::{AgentClient, AgentEvent, ImageData, ImageInput};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

pub struct Bridge {
    feishu: FeishuRestClient,
    agent: Arc<RwLock<AgentClient>>,
    feishu_cfg: FeishuConfig,
    agent_cfg: Arc<AgentConfig>,
    policy: Arc<RwLock<PolicyEngine>>,
    sessions: SessionStore,
    pub bot_open_id: Arc<RwLock<String>>,
    /// Per-chat mutex: serializes abort+prompt to prevent races.
    /// Held only briefly — not during stream processing.
    prompt_locks: RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Per-chat generation counter: incremented on each new prompt.
    /// Streams check this to detect they've been superseded and stop early.
    gen_counters: RwLock<HashMap<String, Arc<AtomicU64>>>,
    /// Dedup: track recently processed message IDs to prevent Feishu redelivery duplicates.
    processed: RwLock<HashSet<String>>,
    /// Whether the current model supports image input.
    image_support: RwLock<bool>,
}

impl Bridge {
    pub async fn new(agent_cfg: Arc<AgentConfig>, feishu_cfg: FeishuConfig) -> Result<Self> {
        let feishu = FeishuRestClient::new(
            feishu_cfg.api_base(),
            &feishu_cfg.app_id,
            &feishu_cfg.app_secret,
        );
        let agent = AgentClient::connect(&agent_cfg.grpc_addr).await?;
        let policy = PolicyEngine::new(feishu_cfg.policy.clone());
        let sessions_dir = dirs_next_path().join("channel").join("feishu");
        let sessions = SessionStore::new(sessions_dir.join("sessions.json"));

        // Fetch bot's own open_id to enable @mention detection in group chats
        let bot_open_id = match feishu.get_bot_info().await {
            Ok(info) => {
                info!("[BOT] open_id={} app_name={}", info.open_id, info.app_name);
                info.open_id
            }
            Err(e) => {
                warn!(
                    "[BOT] Failed to get bot info: {}. @mention detection in groups will not work.",
                    e
                );
                String::new()
            }
        };

        Ok(Self {
            feishu,
            agent: Arc::new(RwLock::new(agent)),
            feishu_cfg,
            agent_cfg,
            policy: Arc::new(RwLock::new(policy)),
            sessions,
            bot_open_id: Arc::new(RwLock::new(bot_open_id)),
            prompt_locks: RwLock::new(HashMap::new()),
            gen_counters: RwLock::new(HashMap::new()),
            processed: RwLock::new(HashSet::new()),
            image_support: RwLock::new(false),
        })
    }

    /// Process an incoming Feishu event.
    pub async fn handle_event(&self, event: FeishuEvent) -> Result<()> {
        let chat_id = match &event.chat_id {
            Some(id) => id.clone(),
            None => {
                warn!("Event without chat_id, skipping");
                return Ok(());
            }
        };
        let sender_id = match &event.sender_open_id {
            Some(id) => id.clone(),
            None => {
                warn!("Event without sender, skipping");
                return Ok(());
            }
        };
        let message_id = match &event.message_id {
            Some(id) => id.clone(),
            None => return Ok(()),
        };

        // Dedup: skip if already processed (Feishu redelivers after 3s without ACK)
        {
            let mut processed = self.processed.write().await;
            if processed.contains(&message_id) {
                info!("[DEDUP] skipping duplicate message_id={}", message_id);
                return Ok(());
            }
            processed.insert(message_id.clone());
            // Keep set bounded: remove oldest if too large
            if processed.len() > 1000 {
                let old: Vec<String> = processed.iter().take(500).cloned().collect();
                for id in old {
                    processed.remove(&id);
                }
            }
        }

        // Skip stale messages replayed on WebSocket reconnect.
        // Feishu replays unacknowledged events on reconnection — without
        // this check, a restart would cause the bot to respond to every
        // historical message in the replay window.
        if let Some(create_ms) = event.create_time_ms {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let age_secs = (now_ms - create_ms) / 1000;
            if age_secs > 60 {
                info!(
                    "[STALE] skipping message_id={} age={}s",
                    message_id, age_secs
                );
                return Ok(());
            }
        }

        let chat_type = event.chat_type.as_deref().unwrap_or("p2p");

        // Log incoming message
        let msg_type = event.msg_type.as_deref().unwrap_or("text");
        let content = event.content.as_deref().unwrap_or("");
        let text_preview = extract_text_content(content, msg_type).unwrap_or_default();
        info!(
            "[RECV] sender={} chat={} chat_type={} msg_type={} text=\"{}\"",
            sender_id,
            if chat_type == "p2p" {
                &*sender_id
            } else {
                chat_id.as_str()
            },
            chat_type,
            msg_type,
            if text_preview.len() > 200 {
                truncate_at_char(&text_preview, 200)
            } else {
                text_preview.to_string()
            },
        );

        // Skip bot's own messages
        let bot_id = self.bot_open_id.read().await.clone();
        if !bot_id.is_empty() && sender_id == bot_id {
            return Ok(());
        }

        // ─── Permission check ──────────────────────────────────────────────
        match chat_type {
            "p2p" => {
                let policy = self.policy.read().await;
                match policy.check_dm(&sender_id) {
                    Access::Denied(reason) => {
                        debug!("[POLICY] DM denied for {}: {}", sender_id, reason);
                        self.feishu
                            .reply_message(
                                &message_id,
                                "text",
                                &serde_json::json!({"text": reason}).to_string(),
                            )
                            .await?;
                        return Ok(());
                    }
                    Access::Allowed => {
                        debug!("[POLICY] DM allowed for {}", sender_id);
                    }
                }
            }
            "group" => {
                let msg_type = event.msg_type.as_deref().unwrap_or("");
                let content = event.content.as_deref().unwrap_or("");
                // Check both content-level mentions (old API) and event-level mentions (API v2)
                let content_mentioned = is_bot_mentioned(content, msg_type, &bot_id);
                let event_mentioned = event
                    .mentions
                    .as_ref()
                    .map(|m| is_bot_mentioned_in_mentions(m, &bot_id))
                    .unwrap_or(false);
                let mentioned = content_mentioned || event_mentioned;
                debug!(
                    "[POLICY] group chat={} mentioned={} (content={} event={}) bot_id={}",
                    chat_id, mentioned, content_mentioned, event_mentioned, bot_id
                );
                // Silently skip non-mentioned messages — no ACK, no reaction
                if !mentioned {
                    return Ok(());
                }
                let policy = self.policy.read().await;
                match policy.check_group(&chat_id, true) {
                    Access::Denied(reason) => {
                        debug!("[POLICY] group denied: {}", reason);
                        self.feishu
                            .reply_message(
                                &message_id,
                                "text",
                                &serde_json::json!({"text": reason}).to_string(),
                            )
                            .await?;
                        return Ok(());
                    }
                    Access::Allowed => {
                        debug!("[POLICY] group allowed for {}", chat_id);
                    }
                }
            }
            _ => {}
        }

        // ─── ACK: react to indicate processing (must complete within 3s) ────
        let ack_reaction_id = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.feishu.react_to_message(&message_id, "Typing"),
        )
        .await
        {
            Ok(Ok(id)) => {
                debug!("[ACK] reaction succeeded: {}", id);
                Some(id)
            }
            Ok(Err(e)) => {
                warn!("[ACK] reaction failed: {}", e);
                None
            }
            Err(_) => {
                warn!("[ACK] reaction timed out after 5s");
                None
            }
        };

        // ─── Handle card action events (approval button clicks) ────────────
        if event.event_type == "card.action.trigger" {
            return self.handle_card_action(&event).await;
        }

        // ─── Extract message content ───────────────────────────────────────
        let msg_type = event.msg_type.as_deref().unwrap_or("text");
        let content = event.content.as_deref().unwrap_or("");

        // ─── Thread handling ──────────────────────────────────────────────
        let thread_id = event.root_id.clone().or(event.parent_id.clone());
        let is_reply = thread_id.is_some();

        // ─── Check for slash commands ─────────────────────────────────────
        if let Some(text) = extract_text_content(content, msg_type) {
            if text.starts_with('/') {
                let result = self
                    .handle_slash_command(
                        &chat_id,
                        thread_id.as_deref(),
                        &message_id,
                        &text,
                        ack_reaction_id,
                    )
                    .await;
                // handle_slash_command handles reactions for non-agent cases.
                // For the _ wildcard (falls through to process_prompt), reactions
                // are managed by the AgentEnd handler in run_prompt_loop.
                return result;
            }
        }

        // ─── Handle different message types ───────────────────────────────
        let ack_rid = ack_reaction_id;
        match msg_type {
            "text" | "post" => {
                let text = extract_text_content(content, msg_type).unwrap_or_default();
                if text.trim().is_empty() {
                    // Image/file only message
                    self.handle_media_message(
                        &chat_id,
                        thread_id.as_deref(),
                        &message_id,
                        &event,
                        ack_rid,
                    )
                    .await?;
                } else {
                    self.process_prompt(
                        &chat_id,
                        thread_id.as_deref(),
                        &message_id,
                        &text,
                        &[],
                        is_reply,
                        ack_rid,
                    )
                    .await?;
                }
            }
            "image" => {
                if let Some(image_key) = extract_image_key(content) {
                    self.handle_image_message(
                        &chat_id,
                        thread_id.as_deref(),
                        &message_id,
                        &image_key,
                        ack_rid,
                    )
                    .await?;
                }
            }
            "file" | "media" | "audio" => {
                self.handle_media_message(
                    &chat_id,
                    thread_id.as_deref(),
                    &message_id,
                    &event,
                    ack_rid,
                )
                .await?;
            }
            _ => {
                info!("Unsupported message type: {}", msg_type);
            }
        }

        Ok(())
    }

    /// Ensure a session exists for this chat/thread, creating one if needed.
    /// Returns the session_id.
    async fn ensure_session(&self, chat_id: &str, thread_id: Option<&str>) -> Result<String> {
        let sid = if let Some(sid) = self.sessions.get(chat_id, thread_id) {
            if !sid.is_empty() {
                // Re-activate session on the agent side (agent may have restarted)
                let mut agent = self.agent.write().await;
                let _ = agent.switch_session(&sid).await;
                drop(agent);
                sid
            } else {
                let mut agent = self.agent.write().await;
                let sid = agent.new_session(&self.agent_cfg.cwd).await?;
                self.sessions.set_session_id(chat_id, thread_id, &sid);
                drop(agent);
                sid
            }
        } else {
            let mut agent = self.agent.write().await;
            let sid = agent.new_session(&self.agent_cfg.cwd).await?;
            self.sessions.set_session_id(chat_id, thread_id, &sid);
            drop(agent);
            sid
        };
        // Always re-apply channel defaults so config changes take effect,
        // including after channel restart when sessions are recreated.
        let mut agent = self.agent.write().await;
        if !self.agent_cfg.model.is_empty() {
            match agent.set_model(&sid, &self.agent_cfg.model).await {
                Ok(()) => tracing::info!("[feishu] set model={}", self.agent_cfg.model),
                Err(e) => tracing::warn!("[feishu] set model failed: {}", e),
            }
        }
        if !self.agent_cfg.thinking_level.is_empty() {
            let _ = agent.set_thinking_level(&sid, &self.agent_cfg.thinking_level).await;
        }
        if !self.agent_cfg.permission_level.is_empty() {
            let _ = agent.set_permission_level(&sid, &self.agent_cfg.permission_level).await;
        }
        let cache = agent
            .get_state(&sid)
            .await
            .map(|s| s.image_support)
            .unwrap_or(false);
        drop(agent);
        *self.image_support.write().await = cache;
        Ok(sid)
    }

    /// Refresh cached image support flag from agent's current model.
    async fn refresh_image_support(&self, session_id: &str) {
        let mut agent = self.agent.write().await;
        if let Ok(state) = agent.get_state(session_id).await {
            *self.image_support.write().await = state.image_support;
        }
    }

    /// Handle slash commands locally (before hitting the agent).
    /// Reactions: for commands handled in-process, swaps "Typing"→"DONE".
    /// For the `_` wildcard (falls through to process_prompt), the AgentEnd
    /// handler in run_prompt_loop manages reactions instead.
    async fn handle_slash_command(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        message_id: &str,
        text: &str,
        ack_reaction_id: Option<String>,
    ) -> Result<()> {
        let parts: Vec<&str> = text.trim().splitn(2, char::is_whitespace).collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match cmd.as_str() {
            "/new" => {
                // Abort the old session if there is one
                if let Some(old_sid) = self.sessions.get(chat_id, thread_id) {
                    if !old_sid.is_empty() {
                        let mut agent = self.agent.write().await;
                        let _ = agent.abort(&old_sid).await;
                    }
                }
                self.sessions.reset(chat_id, thread_id);
                match self.ensure_session(chat_id, thread_id).await {
                    Ok(sid) => {
                        info!("[NEW] session={}", sid);
                        self.feishu
                            .reply_message(
                                message_id,
                                "text",
                                &serde_json::json!({"text": format!("New session: {}", sid)})
                                    .to_string(),
                            )
                            .await?;
                    }
                    Err(e) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Failed to create new session: {}", e)}).to_string()).await?;
                    }
                }
            }

            "/status" if arg.is_empty() => {
                let session_id = self.sessions.get(chat_id, thread_id);
                match session_id {
                    Some(sid) => {
                        let mut agent = self.agent.write().await;
                        match agent.get_state(&sid).await {
                            Ok(state) => {
                                let model_info = match agent.get_available_models(&sid).await {
                                    Ok(models) => models.iter().find(|m| m.id == state.model).map(|m| {
                                        format!("Provider: {}\nImage: {}\nContext: {}K\nMax output: {}",
                                            m.provider,
                                            if m.image { "yes" } else { "no" },
                                            m.context_window / 1000,
                                            if m.max_tokens > 0 { format!("{}K", m.max_tokens / 1000) } else { "unlimited".to_string() },
                                        )
                                    }).unwrap_or_default(),
                                    Err(_) => String::new(),
                                };
                                let image_tag = if state.image_support { " 🖼️" } else { "" };
                                let mi_block = if model_info.is_empty() {
                                    String::new()
                                } else {
                                    format!("\n{}", model_info)
                                };
                                let text = format!(
                                    "Model: {}{}{}\n\nSession: {}\nCWD: {}\nThinking: {}\nQueries: {}\nAuto compaction: {}\n\nContext: {} / {} ({:.1}%)\nTokens: {} in / {} out\nCost: ¥{:.4}",
                                    state.model,
                                    image_tag,
                                    mi_block,
                                    state.session_id,
                                    state.cwd,
                                    state.thinking_level,
                                    state.query_count,
                                    if state.auto_compaction { "on" } else { "off" },
                                    state.context_tokens, state.context_window,
                                    if state.context_window > 0 { (state.context_tokens as f64 / state.context_window as f64) * 100.0 } else { 0.0 },
                                    state.tokens_in, state.tokens_out,
                                    state.total_cost,
                                );
                                self.feishu
                                    .reply_message(
                                        message_id,
                                        "text",
                                        &serde_json::json!({"text": text}).to_string(),
                                    )
                                    .await?;
                            }
                            Err(e) => {
                                self.feishu
                                    .reply_message(
                                        message_id,
                                        "text",
                                        &serde_json::json!({"text": format!("Error: {}", e)})
                                            .to_string(),
                                    )
                                    .await?;
                            }
                        }
                    }
                    None => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": "No active session. Send a message to start."}).to_string()).await?;
                    }
                }
            }

            "/stop" if arg.is_empty() => {
                let session_id = self.sessions.get(chat_id, thread_id);
                if let Some(sid) = session_id {
                    let mut agent = self.agent.write().await;
                    let _ = agent.abort(&sid).await;
                }
                self.feishu
                    .reply_message(
                        message_id,
                        "text",
                        &serde_json::json!({"text": "Stopped."}).to_string(),
                    )
                    .await?;
            }

            "/model" if !arg.is_empty() => {
                // Accept both ":" and "/" as provider-model separator
                let model_id = arg.replace(':', "/");
                let sid = match self.ensure_session(chat_id, thread_id).await {
                    Ok(sid) => sid,
                    Err(e) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Failed to create session: {}", e)}).to_string()).await?;
                        return Ok(());
                    }
                };
                let mut agent = self.agent.write().await;
                match agent.set_model(&sid, &model_id).await {
                    Ok(()) => {
                        drop(agent);
                        self.refresh_image_support(&sid).await;
                        let mut agent = self.agent.write().await;
                        let state = agent.get_state(&sid).await?;
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Model switched to: {}", state.model)}).to_string()).await?;
                    }
                    Err(e) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Failed to switch model: {}", e)}).to_string()).await?;
                    }
                }
            }

            "/models" => {
                let session_id = self.sessions.get(chat_id, thread_id).unwrap_or_default();
                let mut agent = self.agent.write().await;
                match agent.get_available_models(&session_id).await {
                    Ok(models) => {
                        let list: Vec<String> = models
                            .iter()
                            .map(|m| {
                                let image_icon = if m.image { "🖼️ " } else { "" };
                                let ctx = if m.context_window > 0 {
                                    format!(" | {}K ctx", m.context_window / 1000)
                                } else {
                                    String::new()
                                };
                                let out = if m.max_tokens > 0 {
                                    format!(" | {}K out", m.max_tokens / 1000)
                                } else {
                                    String::new()
                                };
                                if m.provider.is_empty() {
                                    format!("• {}{} — `{}`{}{}", image_icon, m.name, m.id, ctx, out)
                                } else {
                                    format!(
                                        "• {}{} — `{}/{}`{}{}",
                                        image_icon, m.name, m.provider, m.id, ctx, out
                                    )
                                }
                            })
                            .collect();
                        let text = if list.is_empty() {
                            "No models available".to_string()
                        } else {
                            format!("Available models ({})\n{}", list.len(), list.join("\n"))
                        };
                        self.feishu
                            .reply_message(
                                message_id,
                                "text",
                                &serde_json::json!({"text": text}).to_string(),
                            )
                            .await?;
                    }
                    Err(e) => {
                        self.feishu
                            .reply_message(
                                message_id,
                                "text",
                                &serde_json::json!({"text": format!("Error: {}", e)}).to_string(),
                            )
                            .await?;
                    }
                }
            }

            "/effort" if !arg.is_empty() => {
                let valid_levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
                let level = arg.to_lowercase();
                if !valid_levels.contains(&level.as_str()) {
                    self.feishu.reply_message(message_id, "text",
                        &serde_json::json!({"text": format!("Invalid level: {}. Use: off, minimal, low, medium, high, xhigh", arg)}).to_string()).await?;
                    return Ok(());
                }
                let sid = match self.ensure_session(chat_id, thread_id).await {
                    Ok(sid) => sid,
                    Err(e) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Failed to create session: {}", e)}).to_string()).await?;
                        return Ok(());
                    }
                };
                let mut agent = self.agent.write().await;
                match agent.set_thinking_level(&sid, &level).await {
                    Ok(()) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Thinking level set to: {}", level)}).to_string()).await?;
                    }
                    Err(e) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Failed to set thinking level: {}", e)}).to_string()).await?;
                    }
                }
            }

            "/compact" => {
                let session_id = self.sessions.get(chat_id, thread_id);
                match session_id {
                    Some(sid) => {
                        let mut agent = self.agent.write().await;
                        match agent.compact(&sid).await {
                            Ok(()) => {
                                self.feishu
                                    .reply_message(
                                        message_id,
                                        "text",
                                        &serde_json::json!({"text": "Context compacted."})
                                            .to_string(),
                                    )
                                    .await?;
                            }
                            Err(e) => {
                                self.feishu.reply_message(message_id, "text",
                                    &serde_json::json!({"text": format!("Failed to compact: {}", e)}).to_string()).await?;
                            }
                        }
                    }
                    None => {
                        self.feishu
                            .reply_message(
                                message_id,
                                "text",
                                &serde_json::json!({"text": "No active session to compact."})
                                    .to_string(),
                            )
                            .await?;
                    }
                }
            }

            "/cwd" if !arg.is_empty() => {
                let session_id = self.sessions.get(chat_id, thread_id).unwrap_or_default();
                let mut agent = self.agent.write().await;
                match agent.set_cwd(&session_id, arg).await {
                    Ok(()) => {
                        self.feishu
                            .reply_message(
                                message_id,
                                "text",
                                &serde_json::json!({"text": format!("CWD set to: {}", arg)})
                                    .to_string(),
                            )
                            .await?;
                    }
                    Err(e) => {
                        self.feishu
                            .reply_message(
                                message_id,
                                "text",
                                &serde_json::json!({"text": format!("Failed to set CWD: {}", e)})
                                    .to_string(),
                            )
                            .await?;
                    }
                }
            }

            "/help" => {
                let help = card::help_card();
                self.feishu
                    .reply_message(message_id, "interactive", &card::card_content(&help))
                    .await?;
            }

            _ => {
                // Unknown slash command — send to agent as normal prompt.
                // Pass ack_reaction_id so the AgentEnd handler can clean it up.
                self.process_prompt(
                    chat_id,
                    thread_id,
                    message_id,
                    text,
                    &[],
                    false,
                    ack_reaction_id,
                )
                .await?;
                // Don't add DONE here — run_prompt_loop's AgentEnd handler does it.
                return Ok(());
            }
        }

        // Non-agent slash commands: swap reactions here since process_prompt wasn't called.
        if let Some(ref rid) = ack_reaction_id {
            let _ = self.feishu.remove_reaction(message_id, rid).await;
        }
        let _ = self.feishu.react_to_message(message_id, "DONE").await;

        Ok(())
    }

    /// Handle an image message.
    async fn handle_image_message(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        message_id: &str,
        image_key: &str,
        ack_reaction_id: Option<String>,
    ) -> Result<()> {
        // Download image, save to disk, and convert to base64
        match self
            .feishu
            .download_resource(message_id, image_key, "image")
            .await
        {
            Ok(data) => {
                let file_path = save_received_file(&data, &format!("image_{}.png", message_id));
                let prompt = format!("[User sent an image: {}]", file_path.display());
                let image_support = *self.image_support.read().await;
                let images = if image_support {
                    let base64 = bytes_to_base64_data(&data, "image/png");
                    vec![ImageInput {
                        content_type: "image_url".into(),
                        data: ImageData::Base64(base64),
                        file_path: Some(file_path.to_string_lossy().to_string()),
                    }]
                } else {
                    vec![]
                };
                self.process_prompt(
                    chat_id,
                    thread_id,
                    message_id,
                    &prompt,
                    &images,
                    false,
                    ack_reaction_id,
                )
                .await?;
            }
            Err(e) => {
                self.feishu
                    .reply_message(
                        message_id,
                        "text",
                        &serde_json::json!({"text": format!("Failed to download image: {}", e)})
                            .to_string(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// Handle a file/media message.
    async fn handle_media_message(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        message_id: &str,
        event: &FeishuEvent,
        ack_reaction_id: Option<String>,
    ) -> Result<()> {
        let content = event.content.as_deref().unwrap_or("");
        let (file_key, file_name) = extract_file_key(content);

        if let Some(key) = file_key {
            let rtype = if event.msg_type.as_deref() == Some("image") {
                "image"
            } else {
                "file"
            };
            match self.feishu.download_resource(message_id, &key, rtype).await {
                Ok(data) => {
                    let name = file_name.unwrap_or_else(|| "file".to_string());
                    let file_path = save_received_file(&data, &name);
                    let text = format!(
                        "[User sent a file: {} ({} bytes)]\nFile path: {}",
                        name,
                        data.len(),
                        file_path.display()
                    );
                    let image_support = *self.image_support.read().await;
                    let images = if image_support {
                        let mime = mime_from_ext(&name);
                        let base64 = bytes_to_base64_data(&data, mime);
                        vec![ImageInput {
                            content_type: "image_url".into(),
                            data: ImageData::Base64(base64),
                            file_path: Some(file_path.to_string_lossy().to_string()),
                        }]
                    } else {
                        vec![]
                    };
                    self.process_prompt(
                        chat_id,
                        thread_id,
                        message_id,
                        &text,
                        &images,
                        false,
                        ack_reaction_id,
                    )
                    .await?;
                }
                Err(e) => {
                    self.feishu
                        .reply_message(
                            message_id,
                            "text",
                            &serde_json::json!({"text": format!("Failed to download file: {}", e)})
                                .to_string(),
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Core prompt processing: get or create session, send prompt, stream response.
    #[allow(clippy::too_many_arguments)]
    async fn process_prompt(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        feishu_msg_id: &str,
        text: &str,
        images: &[ImageInput],
        _is_reply: bool,
        ack_reaction_id: Option<String>,
    ) -> Result<()> {
        // Get or create session — ensure_session also reactivates existing
        // sessions on the agent (switch_session), which is essential after
        // an agent restart.
        let session_id = match self.ensure_session(chat_id, thread_id).await {
            Ok(sid) => sid,
            Err(e) => {
                error!("Failed to ensure session: {}", e);
                self.feishu
                    .reply_message(
                        feishu_msg_id,
                        "text",
                        &serde_json::json!({"text": format!("Failed to create session: {}", e)})
                            .to_string(),
                    )
                    .await?;
                return Ok(());
            }
        };

        self.sessions.touch(chat_id, thread_id);

        // Per-chat lock: serializes abort+prompt (held briefly, not during stream)
        let prompt_lock = {
            let mut locks = self.prompt_locks.write().await;
            locks
                .entry(chat_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };

        // Per-chat generation counter: lets new prompts interrupt old streams
        let gen_counter = {
            let mut counters = self.gen_counters.write().await;
            counters
                .entry(chat_id.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };

        let streaming = self.feishu_cfg.behavior.streaming;
        let feishu = self.feishu.clone();
        let feishu_msg_id = feishu_msg_id.to_string();
        let agent = self.agent.clone();
        let session_id_clone = session_id.clone();
        let images_vec: Vec<ImageInput> = images.to_vec();
        let text_owned = text.to_string();

        // Spawn async task for prompt execution (so Feishu ACK is fast)
        tokio::spawn(async move {
            if let Err(e) = run_prompt_loop(
                &feishu,
                &agent,
                &session_id_clone,
                &feishu_msg_id,
                &text_owned,
                &images_vec,
                streaming,
                &prompt_lock,
                &gen_counter,
                ack_reaction_id,
            )
            .await
            {
                error!("Prompt loop error: {}", e);
                let _ = feishu
                    .reply_message(
                        &feishu_msg_id,
                        "text",
                        &serde_json::json!({"text": format!("Error: {}", e)}).to_string(),
                    )
                    .await;
            }
        });

        Ok(())
    }

    /// Handle card action events (approval button clicks from interactive cards).
    async fn handle_card_action(&self, event: &FeishuEvent) -> Result<()> {
        let content = match &event.content {
            Some(c) => c,
            None => return Ok(()),
        };
        let value: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        let action = value.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let approval_request_id = value
            .get("approval_request_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if approval_request_id.is_empty() {
            return Ok(());
        }

        let approved = action == "approve";
        let note = if approved {
            "approved via Feishu card"
        } else {
            "rejected via Feishu card"
        };

        info!(
            "[CARD_ACTION] action={} approval_request_id={}",
            action, approval_request_id
        );

        // Get any active session to send the decision through.
        // Card actions may arrive after the streaming session has changed,
        // so we try the session associated with the chat.
        let session_id = self
            .sessions
            .get(event.chat_id.as_deref().unwrap_or(""), None)
            .unwrap_or_default();

        if !session_id.is_empty() {
            let mut agent = self.agent.write().await;
            match agent
                .approval_decision(&session_id, approval_request_id, approved, note)
                .await
            {
                Ok(()) => info!("[CARD_ACTION] decision sent successfully"),
                Err(e) => warn!("[CARD_ACTION] failed to send decision: {}", e),
            }
        }

        // Reply to the card message to acknowledge
        if let Some(ref msg_id) = event.message_id {
            let ack_text = if approved {
                "✅ Approved. The tool will execute shortly."
            } else {
                "❌ Rejected. The tool call has been denied."
            };
            self.feishu
                .reply_message(
                    msg_id,
                    "text",
                    &serde_json::json!({"text": ack_text}).to_string(),
                )
                .await?;
        }

        Ok(())
    }

    /// Resolve sender name from Feishu.
    pub async fn resolve_sender_name(&self, open_id: &str) -> Option<String> {
        if !self.feishu_cfg.behavior.resolve_sender_names {
            return None;
        }
        match self.feishu.get_user_info(open_id).await {
            Ok(info) => Some(info.name),
            Err(_) => None,
        }
    }
}

/// Run the prompt → stream → respond loop.
#[allow(clippy::too_many_arguments)]
async fn run_prompt_loop(
    feishu: &FeishuRestClient,
    agent: &Arc<RwLock<AgentClient>>,
    session_id: &str,
    feishu_msg_id: &str,
    text: &str,
    images: &[ImageInput],
    streaming: bool,
    prompt_lock: &tokio::sync::Mutex<()>,
    gen_counter: &AtomicU64,
    ack_reaction_id: Option<String>,
) -> Result<()> {
    // Hold the per-chat lock only for abort+prompt, not during streaming.
    // This lets a new message interrupt an ongoing stream.
    let my_gen = {
        let _guard = prompt_lock.lock().await;

        // Abort current generation, then send the new prompt
        {
            let mut client = agent.write().await;
            let _ = client.abort(session_id).await;
            let mut prompt_text = text.to_string();
            for img in images {
                match &img.data {
                    ImageData::Base64(_) => {
                        if let Some(ref fp) = img.file_path {
                            prompt_text.push_str(&format!("\n[File saved: {}]", fp));
                        } else {
                            prompt_text.push_str("\n[Image attached]");
                        }
                    }
                    ImageData::Url(_) => prompt_text.push_str("\n[Image URL attached]"),
                }
            }
            info!(
                "[SEND] session={} text=\"{}\"",
                session_id,
                if prompt_text.len() > 300 {
                    format!("{}...", truncate_at_char(&prompt_text, 300))
                } else {
                    prompt_text.clone()
                }
            );
            client
                .prompt(session_id, &prompt_text, images.to_vec())
                .await?;
        }

        // Bump generation — we're now the latest active stream
        gen_counter.fetch_add(1, Ordering::SeqCst) + 1
    };
    // Lock released here — streaming happens concurrently with other prompts

    let mut stream = {
        let mut client = agent.write().await;
        client.stream_events(session_id).await?
    };

    let mut stream_text = String::new();
    let mut last_was_content = false;
    // Track per-tool running text for precise ToolEnd replacement (keyed by tool_id).
    // Format in stream_text: "<!--tid:{tool_id}-->🔧 **Running tool:** `{name}`..."
    let mut tool_running: HashMap<String, String> = HashMap::new();
    let streaming_element_id = "stream_out";

    // CardKit state: create card entity, then reply to user with card reference
    let mut cardkit_card_id: Option<String> = None;
    let mut card_seq: u64 = 0;

    // Eagerly create CardKit card + reply as soon as streaming starts
    let mut card_ready = false;

    // Throttle: batch TextChunks to reduce HTTP round-trips (default 150ms)
    let flush_interval = std::time::Duration::from_millis(250);
    let mut last_flush = Instant::now();
    let mut needs_flush = false;

    /// Check if we've been superseded by a newer prompt. If so, stop silently
    /// — the newer stream owns the response.
    macro_rules! check_superseded {
        () => {
            if gen_counter.load(Ordering::SeqCst) != my_gen {
                info!("[STREAM] gen={} superseded, stopping", my_gen);
                return Ok(());
            }
        };
    }

    while let Some(event) = stream.message().await? {
        check_superseded!();

        let parsed = AgentClient::parse_event(event);

        match parsed {
            Some(AgentEvent::AgentStart) | Some(AgentEvent::Ping) => {}
            Some(AgentEvent::ThinkingStart) => {
                last_was_content = false;
                if !stream_text.is_empty() {
                    stream_text.push_str("\n\n");
                }
                stream_text.push_str("💭 **Thinking...**\n\n");
                needs_flush = true;
            }
            Some(AgentEvent::ThinkingDelta(text)) => {
                last_was_content = false;
                stream_text.push_str(&text);
                // Create card on first visible content
                if !card_ready && !stream_text.trim().is_empty() {
                    card_ready = true;
                    let (stream_card, _) = card::streaming_card("");
                    let ck_card = card::to_cardkit_format(&stream_card);
                    match feishu.create_cardkit_card(&ck_card).await {
                        Ok(cid) => {
                            cardkit_card_id = Some(cid.clone());
                            match feishu.reply_with_card_id(feishu_msg_id, &cid).await {
                                Ok(resp) => info!(
                                    "[CARD] reply_with_card_id card_id={} msg_id={}",
                                    cid, resp.message_id
                                ),
                                Err(e) => warn!("[CARD] reply_with_card_id failed: {}", e),
                            }
                        }
                        Err(e) => warn!("[CARD] create_cardkit_card failed: {}", e),
                    }
                }
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    let thinking_flush = std::time::Duration::from_millis(100);
                    if last_flush.elapsed() >= thinking_flush {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ThinkingEnd) => {
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::TextChunk(chunk)) => {
                if !last_was_content && !stream_text.is_empty() {
                    stream_text.push_str("\n\n---\n\n");
                }
                last_was_content = true;
                stream_text.push_str(&chunk);

                if streaming && !card_ready && !stream_text.trim().is_empty() {
                    card_ready = true;
                    let (stream_card, _) = card::streaming_card("");
                    let ck_card = card::to_cardkit_format(&stream_card);
                    match feishu.create_cardkit_card(&ck_card).await {
                        Ok(cid) => {
                            cardkit_card_id = Some(cid.clone());
                            match feishu.reply_with_card_id(feishu_msg_id, &cid).await {
                                Ok(resp) => info!(
                                    "[CARD] reply_with_card_id card_id={} msg_id={}",
                                    cid, resp.message_id
                                ),
                                Err(e) => warn!("[CARD] reply_with_card_id failed: {}", e),
                            }
                        }
                        Err(e) => warn!("[CARD] create_cardkit_card failed: {}", e),
                    }
                }

                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolStart {
                tool_id,
                tool_name,
                tool_args,
                ..
            }) => {
                last_was_content = false;
                let args_preview = tool_args.as_deref().unwrap_or("");
                let args_display = if !args_preview.is_empty() {
                    let truncated = if args_preview.len() > 200 {
                        format!("{}...", truncate_at_char(args_preview, 200))
                    } else {
                        args_preview.to_string()
                    };
                    format!("\n```\n{}\n```", truncated)
                } else {
                    String::new()
                };
                let marker = format!("<!--tid:{}-->", tool_id);
                let running_text = format!(
                    "\n\n{}🔧 **Running tool:** `{}`{}",
                    marker, tool_name, args_display
                );
                tool_running.insert(tool_id.clone(), running_text.clone());
                stream_text.push_str(&running_text);
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolEnd {
                tool_id,
                text: result,
            }) => {
                last_was_content = false;
                if let Some(ref cid) = cardkit_card_id {
                    let (tool_name, old_entry) = {
                        let entry = tool_running.remove(&tool_id);
                        let name = entry
                            .as_ref()
                            .and_then(|s| s.split("**Running tool:** `").nth(1))
                            .and_then(|s| s.split('`').next())
                            .unwrap_or(&tool_id)
                            .to_string();
                        (name, entry)
                    };
                    let result_preview = result.as_deref().unwrap_or("");
                    let result_display = if !result_preview.is_empty() {
                        let truncated = if result_preview.len() > 500 {
                            format!("{}...", truncate_at_char(result_preview, 500))
                        } else {
                            result_preview.to_string()
                        };
                        format!("\n```\n{}\n```", truncated)
                    } else {
                        String::new()
                    };
                    let new_entry = format!(
                        "\n\n✅ **Tool** `{}` **completed**{}",
                        tool_name, result_display
                    );
                    if let Some(ref old) = old_entry {
                        stream_text = stream_text.replace(old, &new_entry);
                    }
                    needs_flush = true;
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolDelta { .. }) => {}
            Some(AgentEvent::ApprovalRequest {
                approval_request_id,
                tool_name,
                risk_level,
                title,
                summary,
                requested_action,
                ..
            }) => {
                // Flush any pending text before sending approval card
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                        needs_flush = false;
                        last_flush = Instant::now();
                    }
                }
                // Finalize current streaming card
                if let Some(ref cid) = cardkit_card_id {
                    let _ = feishu.set_card_streaming_mode(cid, false, card_seq).await;
                    let complete_card = card::complete_card("", &stream_text);
                    let ck_complete = card::to_cardkit_format(&complete_card);
                    card_seq += 1;
                    let _ = feishu
                        .update_cardkit_card(cid, &ck_complete, card_seq)
                        .await;
                    cardkit_card_id = None;
                }
                // Send approval card with Approve/Reject buttons
                let action_preview = if let serde_json::Value::String(s) = &requested_action {
                    s.clone()
                } else {
                    serde_json::to_string_pretty(&requested_action).unwrap_or_default()
                };
                let approval = card::approval_card(
                    &approval_request_id,
                    &tool_name,
                    &risk_level,
                    &title,
                    &summary,
                    &action_preview,
                );
                let ck_card = card::to_cardkit_format(&approval);
                match feishu.create_cardkit_card(&ck_card).await {
                    Ok(cid) => {
                        info!(
                            "[APPROVAL] card created cid={} request_id={}",
                            cid, approval_request_id
                        );
                        card_seq += 1;
                        let _ = feishu.reply_with_card_id(feishu_msg_id, &cid).await;
                    }
                    Err(e) => {
                        warn!("[APPROVAL] create_cardkit_card failed: {}", e);
                        // Fallback: text message
                        let fallback = format!(
                            "⚠️ Approval: {}\nTool: {}\nUse `/approve {}` or `/reject {}` in TUI",
                            title, tool_name, approval_request_id, approval_request_id
                        );
                        feishu
                            .reply_message(
                                feishu_msg_id,
                                "text",
                                &serde_json::json!({"text": fallback}).to_string(),
                            )
                            .await?;
                    }
                }
            }
            Some(AgentEvent::AgentEnd { error }) => {
                check_superseded!();
                // Flush any pending text before finalizing
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu
                            .update_card_element(cid, streaming_element_id, &stream_text, card_seq)
                            .await;
                    }
                }
                // Swap reactions: remove "Typing", add "DONE"
                if let Some(ref rid) = ack_reaction_id {
                    let _ = feishu.remove_reaction(feishu_msg_id, rid).await;
                }
                let _ = feishu.react_to_message(feishu_msg_id, "DONE").await;

                if let Some(err) = error {
                    // "interrupted" is expected when a newer message aborts this
                    // stream — don't show an error card, just let the new stream win.
                    if err.contains("interrupted") || err.contains("Interrupted") {
                        info!("[REPLY] interrupted by newer message, stopping silently");
                    } else {
                        info!("[REPLY] error=\"{}\"", err);
                        let err_card = card::error_card(&err);
                        feishu
                            .reply_message(
                                feishu_msg_id,
                                "interactive",
                                &card::card_content(&err_card),
                            )
                            .await?;
                    }
                } else if !stream_text.trim().is_empty() {
                    info!("[REPLY] text_len={}", stream_text.len());
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        if let Err(e) = feishu.set_card_streaming_mode(cid, false, card_seq).await {
                            warn!("[CARD] set_card_streaming_mode failed: {}", e);
                        }
                        let complete_card = card::complete_card("", &stream_text);
                        let ck_complete = card::to_cardkit_format(&complete_card);
                        card_seq += 1;
                        if let Err(e) = feishu
                            .update_cardkit_card(cid, &ck_complete, card_seq)
                            .await
                        {
                            warn!("[CARD] update_cardkit_card failed: {}", e);
                        }
                    } else {
                        let card = card::complete_card("", &stream_text);
                        feishu
                            .reply_message(feishu_msg_id, "interactive", &card::card_content(&card))
                            .await?;
                    }
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                check_superseded!();
                if let Some(ref rid) = ack_reaction_id {
                    let _ = feishu.remove_reaction(feishu_msg_id, rid).await;
                }
                let _ = feishu.react_to_message(feishu_msg_id, "DONE").await;
                info!("[REPLY] error=\"{}\"", err);
                let err_card = card::error_card(&err);
                feishu
                    .reply_message(feishu_msg_id, "interactive", &card::card_content(&err_card))
                    .await?;
                break;
            }
            None => {}
        }
    }

    Ok(())
}

fn truncate_at_char(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn dirs_next_path() -> std::path::PathBuf {
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".future")
}

/// Save a downloaded file to ~/.future/channel/files/{filename}.
/// Returns the absolute path on success.
fn save_received_file(data: &[u8], filename: &str) -> std::path::PathBuf {
    let dir = dirs_next_path().join("channel").join("files");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(filename);
    let _ = std::fs::write(&path, data);
    path
}
