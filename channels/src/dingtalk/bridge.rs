//! Core bridge logic: DingTalk events → Agent → DingTalk responses.
//! Mirrors the Feishu bridge behavior.

use super::config::DingtalkConfig;
use super::dingtalk_rest::DingtalkRestClient;
use super::dingtalk_ws::{extract_text_content, DingtalkEvent};
use crate::config::AgentConfig;
use crate::grpc_client::{AgentClient, AgentEvent};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub struct DingtalkBridge {
    dingtalk: DingtalkRestClient,
    agent: Arc<RwLock<AgentClient>>,
    agent_cfg: Arc<AgentConfig>,
    /// Per-chat generation counter: incremented on each new prompt.
    gen_counters: RwLock<HashMap<String, Arc<AtomicU64>>>,
    /// Dedup: track recently processed message IDs.
    processed: RwLock<HashSet<String>>,
}

impl DingtalkBridge {
    pub async fn new(agent_cfg: Arc<AgentConfig>, dingtalk_cfg: DingtalkConfig) -> Result<Self> {
        let dingtalk = DingtalkRestClient::new(
            &dingtalk_cfg.domain,
            &dingtalk_cfg.client_id,
            &dingtalk_cfg.client_secret,
        );
        let agent = AgentClient::connect(&agent_cfg.grpc_addr).await?;

        Ok(Self {
            dingtalk,
            agent: Arc::new(RwLock::new(agent)),
            agent_cfg,
            gen_counters: RwLock::new(HashMap::new()),
            processed: RwLock::new(HashSet::new()),
        })
    }

    /// Process an incoming DingTalk event.
    pub async fn handle_event(&self, event: DingtalkEvent) -> Result<()> {
        let chat_id = match &event.chat_id {
            Some(id) => id.clone(),
            None => {
                warn!("Event without chat_id, skipping");
                return Ok(());
            }
        };
        let sender_id = match &event.sender_id {
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

        // Dedup
        {
            let mut processed = self.processed.write().await;
            if processed.contains(&message_id) {
                info!("[DING DEDUP] skipping duplicate message_id={}", message_id);
                return Ok(());
            }
            processed.insert(message_id.clone());
            if processed.len() > 1000 {
                let old: Vec<String> = processed.iter().take(500).cloned().collect();
                for id in old {
                    processed.remove(&id);
                }
            }
        }

        // Skip stale messages (>60s old)
        if let Some(create_ms) = event.create_time_ms {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let age_secs = (now_ms - create_ms) / 1000;
            if age_secs > 60 {
                info!("[DING STALE] skipping message_id={} age={}s", message_id, age_secs);
                return Ok(());
            }
        }

        let msg_type = event.msg_type.as_deref().unwrap_or("text");
        let content = event.content.as_deref().unwrap_or("");
        let text = extract_text_content(content, msg_type).unwrap_or_default();

        info!(
            "[DING RECV] sender={} chat={} msg_type={} text=\"{}\"",
            sender_id, chat_id, msg_type,
            if text.len() > 200 { truncate_at_char(&text, 200) } else { text.clone() },
        );

        // ─── Process prompt ──────────────────────────────────────────
        self.process_prompt(&chat_id, &message_id, &text).await?;

        Ok(())
    }

    /// Core prompt processing: send to agent and stream response.
    async fn process_prompt(
        &self,
        chat_id: &str,
        _msg_id: &str,
        text: &str,
    ) -> Result<()> {
        // Create a new session for each chat (simplified - no session persistence yet)
        let session_id = {
            let mut agent = self.agent.write().await;
            agent.new_session(&self.agent_cfg.cwd).await?
        };

        let agent = self.agent.clone();
        let dingtalk = self.dingtalk.clone();
        let chat_id = chat_id.to_string();
        let _msg_id = _msg_id.to_string();
        let text = text.to_string();

        // Per-chat generation counter
        let gen_counter = {
            let mut counters = self.gen_counters.write().await;
            counters
                .entry(chat_id.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };

        tokio::spawn(async move {
            if let Err(e) = run_prompt_loop(
                &dingtalk,
                &agent,
                &session_id,
                &chat_id,
                &_msg_id,
                &text,
                &gen_counter,
            ).await {
                error!("DingTalk prompt loop error: {}", e);
            }
        });

        Ok(())
    }
}

/// Run the prompt → stream → respond loop.
async fn run_prompt_loop(
    dingtalk: &DingtalkRestClient,
    agent: &Arc<RwLock<AgentClient>>,
    session_id: &str,
    chat_id: &str,
    _msg_id: &str,
    text: &str,
    gen_counter: &AtomicU64,
) -> Result<()> {
    let my_gen = {
        let mut client = agent.write().await;
        let _ = client.abort(session_id).await;
        info!("[DING SEND] session={} text=\"{}\"", session_id,
            if text.len() > 300 { format!("{}...", truncate_at_char(text, 300)) } else { text.to_string() });
        client.prompt(session_id, text, vec![]).await?;
        gen_counter.fetch_add(1, Ordering::SeqCst) + 1
    };

    let mut stream = {
        let mut client = agent.write().await;
        client.stream_events(session_id).await?
    };

    let mut stream_text = String::new();

    macro_rules! check_superseded {
        () => {
            if gen_counter.load(Ordering::SeqCst) != my_gen {
                info!("[DING STREAM] gen={} superseded, stopping", my_gen);
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
                stream_text.push_str("\n\n💭 **Thinking...**\n\n");
            }
            Some(AgentEvent::ThinkingDelta(text)) => {
                stream_text.push_str(&text);
            }
            Some(AgentEvent::ThinkingEnd) => {}
            Some(AgentEvent::TextChunk(chunk)) => {
                stream_text.push_str(&chunk);
            }
            Some(AgentEvent::ToolStart { tool_name, .. }) => {
                stream_text.push_str(&format!("\n\n🔧 **Running:** `{}`\n", tool_name));
            }
            Some(AgentEvent::ToolEnd { text: result, .. }) => {
                if let Some(r) = result {
                    let preview = if r.len() > 500 { truncate_at_char(&r, 500) } else { r.clone() };
                    stream_text.push_str(&format!("\n✅ Tool done: {}\n", preview));
                }
            }
            Some(AgentEvent::AgentEnd { error }) => {
                check_superseded!();
                if let Some(err) = error {
                    if err.contains("interrupted") || err.contains("Interrupted") {
                        info!("[DING REPLY] interrupted, stopping silently");
                    } else {
                        dingtalk.send_text(chat_id, &format!("Error: {}", err)).await?;
                    }
                } else if !stream_text.trim().is_empty() {
                    info!("[DING REPLY] text_len={}", stream_text.len());
                    // Send as markdown reply
                    let preview = if stream_text.len() > 20000 {
                        format!("{}..._(truncated)_", truncate_at_char(&stream_text, 20000))
                    } else {
                        stream_text.clone()
                    };
                    dingtalk.send_text(chat_id, &preview).await?;
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                check_superseded!();
                dingtalk.send_text(chat_id, &format!("Error: {}", err)).await?;
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

fn truncate_at_char(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}
