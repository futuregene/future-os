//! Core bridge logic: DingTalk events → Agent → DingTalk responses.
//! Mirrors the OpenClaw DingTalk connector's webhook-based reply flow.

use super::config::DingtalkConfig;
use super::dingtalk_rest::DingtalkRestClient;
use super::dingtalk_ws::DingtalkEvent;
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
    /// Cached session ID so slash commands can reuse the last prompt session
    /// instead of calling new_session() (which fails when agent is busy).
    session_id: RwLock<Option<String>>,
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
            session_id: RwLock::new(None),
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
        if let Some(ref bot_id) = event.chatbot_user_id {
            if sender_id == *bot_id { return Ok(()); }
        }
        {
            let mut processed = self.processed.write().await;
            if processed.contains(&message_id) {
                return Ok(());
            }
            processed.insert(message_id.clone());
            if processed.len() > 1000 {
                let old: Vec<String> = processed.iter().take(500).cloned().collect();
                for id in old { processed.remove(&id); }
            }
        }
        if let Some(create_ms) = event.create_time_ms {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
            if (now_ms - create_ms) / 1000 > 60 { return Ok(()); }
        }

        let text = event.content.clone().unwrap_or_default();

        info!("[DING RECV] sender={} name={} text=\"{}\"",
            sender_id, event.sender_name.as_deref().unwrap_or("?"),
            if text.len() > 200 { truncate_at_char(&text, 200) } else { text.clone() });

        let webhook = event.session_webhook.clone();

        if text.starts_with('/') {
            self.handle_slash_command(&text, &webhook).await?;
        } else {
            self.process_prompt(&text, webhook).await?;
        }
        Ok(())
    }

    async fn handle_slash_command(&self, text: &str, webhook: &Option<String>) -> Result<()> {
        let parts: Vec<&str> = text.trim().splitn(2, char::is_whitespace).collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");
        let wh = match webhook { Some(w) => w, None => { return Ok(()); } };
        let reply_md = |title: &str, md: &str| {
            let wh2 = wh.to_string();
            let dingtalk = self.dingtalk.clone();
            let title = title.to_string();
            let md = md.to_string();
            tokio::spawn(async move { let _ = dingtalk.reply_webhook_markdown(&wh2, &title, &md).await; });
        };

        match cmd.as_str() {
            "/new" => {
                let mut agent = self.agent.write().await;
                match agent.new_session(&self.agent_cfg.cwd).await {
                    Ok(sid) => {
                        *self.session_id.write().await = Some(sid.clone());
                        reply_md("New Session", &format!("**Session:** `{}`", sid));
                    }
                    Err(e) => reply_md("Error", &format!("**Error:** {}", e)),
                }
            }
            "/status" | "/stop" | "/abort" | "/model" | "/models" | "/compact" | "/effort" => {
                // Reuse cached session (from last prompt) instead of creating a new
                // one — new_session() fails when the agent is busy.
                let sid = match self.get_or_create_session().await {
                    Ok(s) => s,
                    Err(e) => { reply_md("Error", &format!("**Error:** {}", e)); return Ok(()); }
                };
                let mut agent = self.agent.write().await;

                match cmd.as_str() {
                    "/status" => {
                        if let Ok(s) = agent.get_state(&sid).await {
                            let models = agent.get_available_models(&sid).await.unwrap_or_default();
                            let mi = models.iter().find(|m| m.id == s.model).map(|m| format!(
                                "**Provider:** {}\n**Reasoning:** {}\n**Image:** {}\n**Context:** {}K\n**Max output:** {}",
                                m.provider, if m.reasoning { "yes" } else { "no" },
                                if m.image { "yes" } else { "no" },
                                m.context_window / 1000,
                                if m.max_tokens > 0 { format!("{}K", m.max_tokens/1000) } else { "unlimited".into() },
                            )).unwrap_or_default();
                            reply_md("Status", &format!(
                                "**Model:** {}\n{}\n\n**Session:** {}\n**CWD:** {}\n**Thinking:** {}\n**Messages:** {}\n**Auto compaction:** {}\n\n**Context:** {} / {} ({:.1}%)\n**Tokens:** {} in / {} out\n**Cost:** ¥{:.4}",
                                s.model, mi, s.session_id, s.cwd, s.thinking_level, s.message_count,
                                if s.auto_compaction {"on"} else {"off"},
                                s.context_tokens, s.context_window,
                                if s.context_window > 0 { (s.context_tokens as f64 / s.context_window as f64)*100.0 } else { 0.0 },
                                s.tokens_in, s.tokens_out, s.total_cost,
                            ));
                        }
                    }
                    "/stop" | "/abort" => {
                        let _ = agent.abort(&sid).await;
                        reply_md("Stopped", "Stopped.");
                    }
                    "/model" if !arg.is_empty() => {
                        let mid = arg.replace(':', "/");
                        if let Ok(()) = agent.set_model(&sid, &mid).await {
                            if let Ok(s) = agent.get_state(&sid).await {
                                reply_md("Model", &format!("**Model:** `{}`", s.model));
                            }
                        }
                    }
                    "/models" => {
                        if let Ok(models) = agent.get_available_models(&sid).await {
                            let list: Vec<String> = models.iter().map(|m| {
                                let img = if m.image { "🖼️ " } else { "" };
                                format!("• {}{} — `{}/{}`", img, m.name, m.provider, m.id)
                            }).collect();
                            reply_md("Models", &format!("**Models ({})**\n\n{}", list.len(), list.join("\n")));
                        }
                    }
                    "/compact" => {
                        if let Ok(()) = agent.compact(&sid).await { reply_md("Compact", "Context compacted."); }
                    }
                    "/effort" if !arg.is_empty() => {
                        let valid = ["off","minimal","low","medium","high","xhigh"];
                        if !valid.contains(&arg) {
                            reply_md("Invalid", &format!("Invalid: `{}`\n\nUse: `{}`", arg, valid.join(", ")));
                        } else if let Ok(()) = agent.set_thinking_level(&sid, arg).await {
                            reply_md("Thinking", &format!("**Thinking:** `{}`", arg));
                        }
                    }
                    _ => {}
                }
            }
            "/help" => {
                reply_md("Help", "**Commands**\n\n`/new` — new session\n`/status` — session status\n`/stop` — abort prompt\n`/model <id>` — switch model\n`/models` — list models\n`/effort <level>` — thinking level\n`/compact` — compact context\n`/help` — this help");
            }
            _ => {
                self.process_prompt(text, webhook.clone()).await?;
            }
        }
        Ok(())
    }

    /// Return cached session ID (from last prompt), or create a new one as fallback.
    async fn get_or_create_session(&self) -> Result<String> {
        if let Some(ref sid) = *self.session_id.read().await {
            return Ok(sid.clone());
        }
        let mut agent = self.agent.write().await;
        let sid = agent.new_session(&self.agent_cfg.cwd).await?;
        *self.session_id.write().await = Some(sid.clone());
        Ok(sid)
    }

    async fn process_prompt(&self, text: &str, webhook: Option<String>) -> Result<()> {
        let session_id = {
            let mut agent = self.agent.write().await;
            agent.new_session(&self.agent_cfg.cwd).await?
        };
        // Cache for subsequent slash commands
        *self.session_id.write().await = Some(session_id.clone());
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
            Some(AgentEvent::ThinkingStart) => { stream_text.push_str("\n\n> 💭 **Thinking...**\n> \n> "); }
            Some(AgentEvent::ThinkingDelta(t)) => { stream_text.push_str(&t.replace('\n', "\n> ")); }
            Some(AgentEvent::ThinkingEnd) => { stream_text.push_str("\n\n---\n\n"); }
            Some(AgentEvent::TextChunk(chunk)) => { stream_text.push_str(&chunk); }
            Some(AgentEvent::ToolStart { tool_name, .. }) => {
                stream_text.push_str(&format!("\n\n🔧 **{}**\n\n```\n", tool_name));
            }
            Some(AgentEvent::ToolEnd { text: result, .. }) => {
                if let Some(r) = result {
                    let preview = truncate_tool_output(&r);
                    stream_text.push_str(&preview);
                }
                stream_text.push_str("\n```\n");
            }
            Some(AgentEvent::AgentEnd { error }) => {
                check_superseded!();
                if let Some(err) = error {
                    if !err.contains("interrupted") && !err.contains("Interrupted") {
                        if let Some(ref wh) = webhook {
                            dingtalk.reply_webhook_markdown(wh, "Error", &format!("**Error:** {}", err)).await?;
                        }
                    }
                } else if !stream_text.trim().is_empty() {
                    if let Some(ref wh) = webhook {
                        let preview = if stream_text.len() > 20000 {
                            format!("{}...", truncate_at_char(&stream_text, 20000))
                        } else { stream_text.clone() };
                        dingtalk.reply_webhook_markdown(wh, "Future OS", &preview).await?;
                    }
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                check_superseded!();
                if let Some(ref wh) = webhook { dingtalk.reply_webhook_markdown(wh, "Error", &format!("**Error:** {}", err)).await?; }
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

/// Truncate tool output to max 5 lines or 500 chars (Unicode-aware), whichever is smaller.
fn truncate_tool_output(s: &str) -> String {
    const MAX_LINES: usize = 5;
    const MAX_CHARS: usize = 500;

    let char_count = s.chars().count();
    let line_count = s.lines().count();

    if line_count <= MAX_LINES && char_count <= MAX_CHARS {
        return s.to_string();
    }

    let mut truncated = String::new();
    let mut lines = 0;
    let mut chars = 0;

    for ch in s.chars() {
        if ch == '\n' {
            lines += 1;
            if lines >= MAX_LINES {
                break;
            }
        }
        truncated.push(ch);
        chars += 1;
        if chars >= MAX_CHARS {
            break;
        }
    }

    truncated.push_str("...\n_(truncated)_");
    truncated
}
