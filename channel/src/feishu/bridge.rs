//! Core bridge logic: Feishu events → Agent → Feishu responses.
//!
//! Orchestrates the flow: receive Feishu event → check permissions →
//! handle slash commands locally → prompt agent via gRPC →
//! stream response back to Feishu via cards.

use super::card;
use super::config::FeishuConfig;
use super::feishu_rest::{
    bytes_to_base64_data, mime_from_ext, FeishuRestClient,
};
use super::feishu_ws::{extract_file_key, extract_image_key, extract_text_content, is_bot_mentioned, is_bot_mentioned_in_mentions, FeishuEvent};
use super::policy::{Access, PolicyEngine};
use super::session_store::SessionStore;
use crate::config::AgentConfig;
use crate::grpc_client::{AgentClient, AgentEvent, ImageData, ImageInput};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
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
    /// Per-chat mutex to serialize prompt calls (abort + prompt must be atomic).
    prompt_locks: RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Dedup: track recently processed message IDs to prevent Feishu redelivery duplicates.
    processed: RwLock<HashSet<String>>,
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
                warn!("[BOT] Failed to get bot info: {}. @mention detection in groups will not work.", e);
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
            processed: RwLock::new(HashSet::new()),
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
                info!("[STALE] skipping message_id={} age={}s", message_id, age_secs);
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
            if chat_type == "p2p" { &*sender_id } else { chat_id.as_str() },
            chat_type,
            msg_type,
            if text_preview.len() > 200 { truncate_at_char(&text_preview, 200) } else { text_preview.to_string() },
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
                        self.feishu.reply_message(&message_id, "text",
                            &serde_json::json!({"text": reason}).to_string()).await?;
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
                let event_mentioned = event.mentions.as_ref()
                    .map(|m| is_bot_mentioned_in_mentions(m, &bot_id))
                    .unwrap_or(false);
                let mentioned = content_mentioned || event_mentioned;
                debug!("[POLICY] group chat={} mentioned={} (content={} event={}) bot_id={}",
                    chat_id, mentioned, content_mentioned, event_mentioned, bot_id);
                // Silently skip non-mentioned messages — no ACK, no reaction
                if !mentioned {
                    return Ok(());
                }
                let policy = self.policy.read().await;
                match policy.check_group(&chat_id, true) {
                    Access::Denied(reason) => {
                        debug!("[POLICY] group denied: {}", reason);
                        self.feishu.reply_message(&message_id, "text",
                            &serde_json::json!({"text": reason}).to_string()).await?;
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

        // ─── Extract message content ───────────────────────────────────────
        let msg_type = event.msg_type.as_deref().unwrap_or("text");
        let content = event.content.as_deref().unwrap_or("");

        // ─── Thread handling ──────────────────────────────────────────────
        let thread_id = event.root_id.clone().or(event.parent_id.clone());
        let is_reply = thread_id.is_some();

        // ─── Check for slash commands ─────────────────────────────────────
        if let Some(text) = extract_text_content(content, msg_type) {
            if text.starts_with('/') {
                return self.handle_slash_command(&chat_id, thread_id.as_deref(), &message_id, &text).await;
            }
        }

        // ─── Handle different message types ───────────────────────────────
        let ack_rid = ack_reaction_id;
        match msg_type {
            "text" | "post" => {
                let text = extract_text_content(content, msg_type).unwrap_or_default();
                if text.trim().is_empty() {
                    // Image/file only message
                    self.handle_media_message(&chat_id, thread_id.as_deref(), &message_id, &event, ack_rid).await?;
                } else {
                    self.process_prompt(&chat_id, thread_id.as_deref(), &message_id, &text, &[], is_reply, ack_rid).await?;
                }
            }
            "image" => {
                if let Some(image_key) = extract_image_key(content) {
                    self.handle_image_message(&chat_id, thread_id.as_deref(), &message_id, &image_key, ack_rid).await?;
                }
            }
            "file" | "media" | "audio" => {
                self.handle_media_message(&chat_id, thread_id.as_deref(), &message_id, &event, ack_rid).await?;
            }
            _ => {
                info!("Unsupported message type: {}", msg_type);
            }
        }

        Ok(())
    }

    /// Handle slash commands locally (before hitting the agent).
    async fn handle_slash_command(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let parts: Vec<&str> = text.trim().splitn(2, char::is_whitespace).collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match cmd.as_str() {
            "/reset" => {
                self.sessions.reset(chat_id, thread_id);
                self.feishu.reply_message(message_id, "text",
                    &serde_json::json!({"text": "Session reset. Start a new conversation."}).to_string()).await?;
            }

            "/status" => {
                let session_id = self.sessions.get(chat_id, thread_id);
                match session_id {
                    Some(sid) => {
                        let mut agent = self.agent.write().await;
                        match agent.get_state(&sid).await {
                            Ok(state) => {
                                let status = card::status_card(
                                    &state.model, &state.thinking_level,
                                    state.tokens_in, state.tokens_out, state.message_count,
                                );
                                self.feishu.reply_message(message_id, "interactive",
                                    &card::card_content(&status)).await?;
                            }
                            Err(e) => {
                                self.feishu.reply_message(message_id, "text",
                                    &serde_json::json!({"text": format!("Error: {}", e)}).to_string()).await?;
                            }
                        }
                    }
                    None => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": "No active session. Send a message to start."}).to_string()).await?;
                    }
                }
            }

            "/model" if !arg.is_empty() => {
                let session_id = self.sessions.get(chat_id, thread_id);
                match session_id {
                    Some(sid) => {
                        let mut agent = self.agent.write().await;
                        match agent.set_model(&sid, arg).await {
                            Ok(()) => {
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
                    None => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": "No active session. Send a message first."}).to_string()).await?;
                    }
                }
            }

            "/models" => {
                let session_id = self.sessions.get(chat_id, thread_id).unwrap_or_default();
                let mut agent = self.agent.write().await;
                match agent.get_available_models(&session_id).await {
                    Ok(models) => {
                        let list: Vec<String> = models.iter().map(|m| format!("• {} ({})", m.name, m.id)).collect();
                        let text = if list.is_empty() {
                            "No models available".to_string()
                        } else {
                            format!("Available models:\n{}", list.join("\n"))
                        };
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": text}).to_string()).await?;
                    }
                    Err(e) => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": format!("Error: {}", e)}).to_string()).await?;
                    }
                }
            }

            "/abort" => {
                let session_id = self.sessions.get(chat_id, thread_id);
                match session_id {
                    Some(sid) => {
                        let mut agent = self.agent.write().await;
                        match agent.abort(&sid).await {
                            Ok(()) => {
                                self.feishu.reply_message(message_id, "text",
                                    &serde_json::json!({"text": "Aborted current generation."}).to_string()).await?;
                            }
                            Err(e) => {
                                self.feishu.reply_message(message_id, "text",
                                    &serde_json::json!({"text": format!("Failed to abort: {}", e)}).to_string()).await?;
                            }
                        }
                    }
                    None => {
                        self.feishu.reply_message(message_id, "text",
                            &serde_json::json!({"text": "No active session to abort."}).to_string()).await?;
                    }
                }
            }

            "/help" => {
                let help = card::help_card();
                self.feishu.reply_message(message_id, "interactive",
                    &card::card_content(&help)).await?;
            }

            _ => {
                // Unknown slash command — send to agent as normal prompt
                self.process_prompt(chat_id, thread_id, message_id, text, &[], false, None).await?;
            }
        }

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
        // Download image and convert to base64
        match self.feishu.download_resource(message_id, image_key, "image").await {
            Ok(data) => {
                let base64 = bytes_to_base64_data(&data, "image/png");
                let prompt = "[User sent an image]";
                let images = vec![ImageInput {
                    content_type: "image_url".into(),
                    data: ImageData::Base64(base64),
                }];
                self.process_prompt(chat_id, thread_id, message_id, prompt, &images, false, ack_reaction_id).await?;
            }
            Err(e) => {
                self.feishu.reply_message(message_id, "text",
                    &serde_json::json!({"text": format!("Failed to download image: {}", e)}).to_string()).await?;
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
            let rtype = if event.msg_type.as_deref() == Some("image") { "image" } else { "file" };
            match self.feishu.download_resource(message_id, &key, rtype).await {
                Ok(data) => {
                    let name = file_name.unwrap_or_else(|| "file".to_string());
                    let mime = mime_from_ext(&name);
                    let base64 = bytes_to_base64_data(&data, mime);
                    let text = format!("[User sent a file: {} ({} bytes)]", name, data.len());
                    let images = vec![ImageInput {
                        content_type: "image_url".into(),
                        data: ImageData::Base64(base64),
                    }];
                    self.process_prompt(chat_id, thread_id, message_id, &text, &images, false, ack_reaction_id).await?;
                }
                Err(e) => {
                    self.feishu.reply_message(message_id, "text",
                        &serde_json::json!({"text": format!("Failed to download file: {}", e)}).to_string()).await?;
                }
            }
        }
        Ok(())
    }

    /// Core prompt processing: get or create session, send prompt, stream response.
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
        // Get or create session
        let (mut session_id, is_new) = self.sessions.get_or_create(chat_id, thread_id);

        if is_new || session_id.is_empty() {
            // Create a new agent session
            let mut agent = self.agent.write().await;
            match agent.new_session(&self.agent_cfg.cwd).await {
                Ok(sid) => {
                    session_id = sid;
                    self.sessions.set_session_id(chat_id, thread_id, &session_id);
                }
                Err(e) => {
                    error!("Failed to create agent session: {}", e);
                    self.feishu.reply_message(feishu_msg_id, "text",
                        &serde_json::json!({"text": format!("Failed to create session: {}", e)}).to_string()).await?;
                    return Ok(());
                }
            }
        }

        self.sessions.touch(chat_id, thread_id);

        // Per-chat lock: serialize abort+prompt to prevent "still streaming" errors
        let prompt_lock = {
            let mut locks = self.prompt_locks.write().await;
            locks.entry(chat_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
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
                prompt_lock,
                ack_reaction_id,
            ).await {
                error!("Prompt loop error: {}", e);
                let _ = feishu.reply_message(&feishu_msg_id, "text",
                    &serde_json::json!({"text": format!("Error: {}", e)}).to_string()).await;
            }
        });

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
async fn run_prompt_loop(
    feishu: &FeishuRestClient,
    agent: &Arc<RwLock<AgentClient>>,
    session_id: &str,
    feishu_msg_id: &str,
    text: &str,
    images: &[ImageInput],
    streaming: bool,
    prompt_lock: Arc<tokio::sync::Mutex<()>>,
    ack_reaction_id: Option<String>,
) -> Result<()> {
    // Hold the per-chat lock for the entire prompt + stream lifecycle.
    // This prevents a second run_prompt_loop from starting (and aborting
    // this one) while we're still streaming, which would produce duplicate
    // replies from both the aborted stream and the new stream.
    let _guard = prompt_lock.lock().await;

    // Send prompt via gRPC
    {
        let mut client = agent.write().await;
        let _ = client.abort(session_id).await;
        let mut prompt_text = text.to_string();
        for img in images {
            match &img.data {
                ImageData::Base64(_) => prompt_text.push_str("\n[Image attached]"),
                ImageData::Url(_) => prompt_text.push_str("\n[Image URL attached]"),
            }
        }
        info!("[SEND] session={} text=\"{}\"", session_id,
            if prompt_text.len() > 300 { format!("{}...", truncate_at_char(&prompt_text, 300)) } else { prompt_text.clone() });
        client.prompt(session_id, &prompt_text, images.to_vec()).await?;
    }

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

    while let Some(event) = stream.message().await? {
        let parsed = AgentClient::parse_event(event);

        match parsed {
            Some(AgentEvent::AgentStart) | Some(AgentEvent::Ping) => {}
            Some(AgentEvent::ThinkingStart) => {
                last_was_content = false;
                if !stream_text.is_empty() {
                    stream_text.push_str("\n\n");
                }
                stream_text.push_str("💭 **Thinking...**\n\n");
                // Don't flush immediately — let the throttle handle it to avoid
                // blocking the gRPC event stream on an HTTP round-trip.
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
                                Ok(resp) => info!("[CARD] reply_with_card_id card_id={} msg_id={}", cid, resp.message_id),
                                Err(e) => warn!("[CARD] reply_with_card_id failed: {}", e),
                            }
                        }
                        Err(e) => warn!("[CARD] create_cardkit_card failed: {}", e),
                    }
                }
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    // Use a shorter flush interval for thinking (100ms vs 250ms)
                    // because thinking tokens are small and rapid — a 250ms
                    // batch looks stuttery compared to content chunks.
                    let thinking_flush = std::time::Duration::from_millis(100);
                    if last_flush.elapsed() >= thinking_flush {
                        card_seq += 1;
                        let _ = feishu.update_card_element(cid, streaming_element_id, &stream_text, card_seq).await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ThinkingEnd) => {
                // Flush pending thinking text
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu.update_card_element(cid, streaming_element_id, &stream_text, card_seq).await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::TextChunk(chunk)) => {
                // Add divider when transitioning from thinking/tools back to content
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
                                Ok(resp) => info!("[CARD] reply_with_card_id card_id={} msg_id={}", cid, resp.message_id),
                                Err(e) => warn!("[CARD] reply_with_card_id failed: {}", e),
                            }
                        }
                        Err(e) => warn!("[CARD] create_cardkit_card failed: {}", e),
                    }
                }

                // Throttled stream update: flush at most every 250ms
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu.update_card_element(cid, streaming_element_id, &stream_text, card_seq).await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolStart { tool_id, tool_name, tool_args, .. }) => {
                last_was_content = false;
                // Append tool call in chronological order
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
                // Store full entry (marker + text) for exact ToolEnd replacement
                tool_running.insert(tool_id.clone(), running_text.clone());
                stream_text.push_str(&running_text);
                // Tool start: throttle flush (same 250ms interval as content)
                needs_flush = true;
                if let Some(ref cid) = cardkit_card_id {
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu.update_card_element(cid, streaming_element_id, &stream_text, card_seq).await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolEnd { tool_id, text: result }) => {
                last_was_content = false;
                if let Some(ref cid) = cardkit_card_id {
                    let (tool_name, old_entry) = {
                        let entry = tool_running.remove(&tool_id);
                        let name = entry.as_ref()
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
                    // Exact replacement using marker+text
                    if let Some(ref old) = old_entry {
                        stream_text = stream_text.replace(old, &new_entry);
                    }
                    // Tool end: throttle flush (same 250ms interval as content)
                    needs_flush = true;
                    if last_flush.elapsed() >= flush_interval {
                        card_seq += 1;
                        let _ = feishu.update_card_element(cid, streaming_element_id, &stream_text, card_seq).await;
                        last_flush = Instant::now();
                        needs_flush = false;
                    }
                }
            }
            Some(AgentEvent::ToolDelta { .. }) => {}
            Some(AgentEvent::AgentEnd { error }) => {
                // Flush any pending text before finalizing
                if needs_flush {
                    if let Some(ref cid) = cardkit_card_id {
                        card_seq += 1;
                        let _ = feishu.update_card_element(cid, streaming_element_id, &stream_text, card_seq).await;
                    }
                }
                // Swap reactions: remove "Typing", add "DONE"
                if let Some(ref rid) = ack_reaction_id {
                    let _ = feishu.remove_reaction(feishu_msg_id, rid).await;
                }
                let _ = feishu.react_to_message(feishu_msg_id, "DONE").await;

                if let Some(err) = error {
                    info!("[REPLY] error=\"{}\"", err);
                    let err_card = card::error_card(&err);
                    feishu.reply_message(feishu_msg_id, "interactive",
                        &card::card_content(&err_card)).await?;
                } else if !stream_text.trim().is_empty() {
                    info!("[REPLY] text_len={}", stream_text.len());
                    if let Some(ref cid) = cardkit_card_id {
                        // Disable streaming mode FIRST, then update final card content.
                        // Order matters: Feishu uses streaming_mode to drive the
                        // "[生成中...]" status in the message list.
                        card_seq += 1;
                        if let Err(e) = feishu.set_card_streaming_mode(cid, false, card_seq).await {
                            warn!("[CARD] set_card_streaming_mode failed: {}", e);
                        }
                        let complete_card = card::complete_card("", &stream_text);
                        let ck_complete = card::to_cardkit_format(&complete_card);
                        card_seq += 1;
                        if let Err(e) = feishu.update_cardkit_card(cid, &ck_complete, card_seq).await {
                            warn!("[CARD] update_cardkit_card failed: {}", e);
                        }
                    } else {
                        let card = card::complete_card("", &stream_text);
                        feishu.reply_message(feishu_msg_id, "interactive",
                            &card::card_content(&card)).await?;
                    }
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                // Swap reactions even on error
                if let Some(ref rid) = ack_reaction_id {
                    let _ = feishu.remove_reaction(feishu_msg_id, rid).await;
                }
                let _ = feishu.react_to_message(feishu_msg_id, "DONE").await;
                info!("[REPLY] error=\"{}\"", err);
                let err_card = card::error_card(&err);
                feishu.reply_message(feishu_msg_id, "interactive",
                    &card::card_content(&err_card)).await?;
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
