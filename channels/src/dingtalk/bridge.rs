//! Core bridge logic: DingTalk events → Agent → DingTalk responses.
//! Mirrors the OpenClaw DingTalk connector's webhook-based reply flow.

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
    gen_counters: RwLock<HashMap<String, Arc<AtomicU64>>>,
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

    pub async fn handle_event(&self, event: DingtalkEvent) -> Result<()> {
        let sender_id = match &event.sender_id {
            Some(id) => id.clone(),
            None => { warn!("Event without sender, skipping"); return Ok(()); }
        };
        let message_id = match &event.message_id {
            Some(id) => id.clone(),
            None => return Ok(()),
        };

        // Skip bot's own messages
        if let Some(ref bot_id) = event.chatbot_user_id {
            if sender_id == *bot_id { return Ok(()); }
        }

        // Dedup
        {
            let mut processed = self.processed.write().await;
            if processed.contains(&message_id) {
                info!("[DING DEDUP] skip {}", message_id);
                return Ok(());
            }
            processed.insert(message_id.clone());
            if processed.len() > 1000 {
                let old: Vec<String> = processed.iter().take(500).cloned().collect();
                for id in old { processed.remove(&id); }
            }
        }

        // Skip stale messages
        if let Some(create_ms) = event.create_time_ms {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
            if (now_ms - create_ms) / 1000 > 60 {
                info!("[DING STALE] skip {}", message_id);
                return Ok(());
            }
        }

        let text = extract_text_content(
            event.content.as_deref().unwrap_or(""),
            event.msg_type.as_deref().unwrap_or("text"),
        ).unwrap_or_default();

        info!("[DING RECV] sender={} name={} text=\"{}\" webhook={}",
            sender_id,
            event.sender_name.as_deref().unwrap_or("?"),
            if text.len() > 200 { truncate_at_char(&text, 200) } else { text.clone() },
            event.session_webhook.as_deref().unwrap_or("none"),
        );

        let webhook = event.session_webhook.clone();
        self.process_prompt(&text, webhook).await?;
        Ok(())
    }

    async fn process_prompt(&self, text: &str, webhook: Option<String>) -> Result<()> {
        let session_id = {
            let mut agent = self.agent.write().await;
            agent.new_session(&self.agent_cfg.cwd).await?
        };

        let agent = self.agent.clone();
        let dingtalk = self.dingtalk.clone();
        let text = text.to_string();

        let gen_counter = {
            let mut counters = self.gen_counters.write().await;
            counters.entry("global".into()).or_insert_with(|| Arc::new(AtomicU64::new(0))).clone()
        };

        tokio::spawn(async move {
            if let Err(e) = run_prompt_loop(&dingtalk, &agent, &session_id, &text, &gen_counter, webhook).await {
                error!("DingTalk prompt loop error: {}", e);
            }
        });

        Ok(())
    }
}

async fn run_prompt_loop(
    dingtalk: &DingtalkRestClient,
    agent: &Arc<RwLock<AgentClient>>,
    session_id: &str,
    text: &str,
    gen_counter: &AtomicU64,
    webhook: Option<String>,
) -> Result<()> {
    let my_gen = {
        let mut client = agent.write().await;
        let _ = client.abort(session_id).await;
        info!("[DING SEND] session={} text=\"{}\"", session_id,
            if text.len() > 300 { truncate_at_char(text, 300) } else { text.to_string() });
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
            Some(AgentEvent::ThinkingDelta(t)) => { stream_text.push_str(&t); }
            Some(AgentEvent::ThinkingEnd) => {}
            Some(AgentEvent::TextChunk(chunk)) => { stream_text.push_str(&chunk); }
            Some(AgentEvent::ToolStart { tool_name, .. }) => {
                stream_text.push_str(&format!("\n\n🔧 `{}`\n", tool_name));
            }
            Some(AgentEvent::ToolEnd { text: result, .. }) => {
                if let Some(r) = result {
                    let preview = if r.len() > 500 { truncate_at_char(&r, 500) } else { r };
                    stream_text.push_str(&format!("\n✅ {}\n", preview));
                }
            }
            Some(AgentEvent::AgentEnd { error }) => {
                check_superseded!();
                if let Some(err) = error {
                    if !err.contains("interrupted") && !err.contains("Interrupted") {
                        if let Some(ref wh) = webhook {
                            dingtalk.reply_webhook(wh, &format!("Error: {}", err)).await?;
                        }
                    }
                } else if !stream_text.trim().is_empty() {
                    if let Some(ref wh) = webhook {
                        let preview = if stream_text.len() > 20000 {
                            format!("{}...", truncate_at_char(&stream_text, 20000))
                        } else { stream_text.clone() };
                        dingtalk.reply_webhook(wh, &preview).await?;
                    }
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                check_superseded!();
                if let Some(ref wh) = webhook {
                    dingtalk.reply_webhook(wh, &format!("Error: {}", err)).await?;
                }
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
