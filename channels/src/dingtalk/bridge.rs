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
    /// One Agent session per DingTalk conversation. Sharing a single cached
    /// session across chats would leak context; creating one per message would
    /// make supersede_session a no-op.
    session_ids: RwLock<HashMap<String, String>>,
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
            session_ids: RwLock::new(HashMap::new()),
        })
    }

    pub async fn handle_event(&self, event: DingtalkEvent) -> Result<()> {
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
        if let Some(ref bot_id) = event.chatbot_user_id {
            if sender_id == *bot_id {
                return Ok(());
            }
        }
        {
            let mut processed = self.processed.write().await;
            if processed.contains(&message_id) {
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
        if let Some(create_ms) = event.create_time_ms {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            if (now_ms - create_ms) / 1000 > 60 {
                return Ok(());
            }
        }

        let text = event.content.clone().unwrap_or_default();

        // Hoisted out of the info! args: the macro only evaluates its
        // arguments when a subscriber is installed, and this truncation must
        // run regardless (also exercised directly in tests).
        let text_preview = if text.len() > 200 {
            truncate_at_char(&text, 200)
        } else {
            text.clone()
        };
        info!(
            "[DING RECV] sender={} name={} text=\"{}\"",
            sender_id,
            event.sender_name.as_deref().unwrap_or("?"),
            text_preview
        );

        let webhook = event.session_webhook.clone();
        let conversation_key = event
            .chat_id
            .clone()
            .unwrap_or(format!("sender:{sender_id}"));

        if text.starts_with('/') {
            self.handle_slash_command(&text, &webhook, &conversation_key)
                .await?;
        } else {
            self.process_prompt(&text, webhook, &conversation_key)
                .await?;
        }
        Ok(())
    }

    async fn handle_slash_command(
        &self,
        text: &str,
        webhook: &Option<String>,
        conversation_key: &str,
    ) -> Result<()> {
        let parts: Vec<&str> = text.trim().splitn(2, char::is_whitespace).collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");
        let wh = match webhook {
            Some(w) => w,
            None => {
                return Ok(());
            }
        };
        let reply_md = |title: &str, md: &str| {
            let wh2 = wh.to_string();
            let dingtalk = self.dingtalk.clone();
            let title = title.to_string();
            let md = md.to_string();
            tokio::spawn(async move {
                let _ = dingtalk.reply_webhook_markdown(&wh2, &title, &md).await;
            });
        };

        match cmd.as_str() {
            "/new" => {
                // Abort current session (if any) then create a fresh one.
                // Hold agent lock once to avoid "agent is busy" from an
                // in-flight prompt.
                let old_sid = self.session_ids.read().await.get(conversation_key).cloned();
                let mut agent = self.agent.write().await;
                if let Some(ref sid) = old_sid {
                    let _ = agent.abort(sid).await;
                }
                match agent.new_session(&self.agent_cfg.cwd, "dingtalk").await {
                    Ok(sid) => {
                        self.session_ids
                            .write()
                            .await
                            .insert(conversation_key.to_string(), sid.clone());
                        reply_md("New Session", &format!("**Session:** `{}`", sid));
                    }
                    Err(e) => reply_md("Error", &format!("**Error:** {}", e)),
                }
            }
            "/status" | "/stop" | "/model" | "/models" | "/compact" | "/effort" | "/cwd" => {
                // Reuse cached session (from last prompt) instead of creating a new
                // one — new_session() fails when the agent is busy.
                let sid = match self.get_or_create_session(conversation_key).await {
                    Ok(s) => s,
                    Err(e) => {
                        reply_md("Error", &format!("**Error:** {}", e));
                        return Ok(());
                    }
                };
                let mut agent = self.agent.write().await;

                match cmd.as_str() {
                    "/status" => {
                        if let Ok(s) = agent.get_state(&sid).await {
                            let models = agent.get_available_models(&sid).await.unwrap_or_default();
                            let mi = models.iter().find(|m| m.id == s.model).map(|m| format!(
                                "**Provider:** {}\n\n**Image:** {}\n\n**Context:** {}K\n\n**Max output:** {}",
                                m.provider,
                                if m.image { "yes" } else { "no" },
                                m.context_window / 1000,
                                if m.max_tokens > 0 { format!("{}K", m.max_tokens/1000) } else { "unlimited".into() },
                            )).unwrap_or_default();
                            reply_md("Status", &format!(
                                "**Model:** {}\n\n{}\n\n**Session:** {}\n\n**CWD:** {}\n\n**Thinking:** {}\n\n**Queries:** {}\n\n**Auto compaction:** {}\n\n**Context:** {} / {} ({:.1}%)\n\n**Tokens:** {} in / {} out\n\n**Cost:** ¥{:.4}",
                                s.model, mi, s.session_id, s.cwd, s.thinking_level, s.query_count,
                                if s.auto_compaction {"on"} else {"off"},
                                s.context_tokens, s.context_window,
                                if s.context_window > 0 { (s.context_tokens as f64 / s.context_window as f64)*100.0 } else { 0.0 },
                                s.tokens_in, s.tokens_out, s.total_cost,
                            ));
                        }
                    }
                    "/stop" => {
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
                            let list: Vec<String> = models
                                .iter()
                                .map(|m| {
                                    let img = if m.image { "🖼️ " } else { "" };
                                    let ctx = if m.context_window > 0 {
                                        format!(" | {}K ctx", m.context_window / 1000)
                                    } else {
                                        String::new()
                                    };
                                    // max_tokens is not in the list_models wire
                                    // response (the client reports 0).
                                    format!(
                                        "• {}{} — `{}/{}`{}",
                                        img, m.name, m.provider, m.id, ctx
                                    )
                                })
                                .collect();
                            reply_md(
                                "Models",
                                &format!("**Models ({})**\n\n{}", list.len(), list.join("\n\n")),
                            );
                        }
                    }
                    "/compact" => {
                        if let Ok(()) = agent.compact(&sid).await {
                            reply_md("Compact", "Context compacted.");
                        }
                    }
                    "/effort" if !arg.is_empty() => {
                        let valid = ["off", "minimal", "low", "medium", "high", "xhigh"];
                        if !valid.contains(&arg) {
                            reply_md(
                                "Invalid",
                                &format!("Invalid: `{}`\n\nUse: `{}`", arg, valid.join(", ")),
                            );
                        } else if let Ok(()) = agent.set_thinking_level(&sid, arg).await {
                            reply_md("Thinking", &format!("**Thinking:** `{}`", arg));
                        }
                    }
                    "/cwd" if !arg.is_empty() => {
                        if let Ok(()) = agent.set_cwd(&sid, arg).await {
                            reply_md("CWD", &format!("**CWD:** `{}`", arg));
                        }
                    }
                    _ => {}
                }
            }
            "/help" => {
                reply_md("Help", "**Commands**\n\n`/new` — new session\n\n`/status` — session status\n\n`/stop` — abort prompt\n\n`/model <id>` — switch model\n\n`/models` — list models\n\n`/effort <level>` — thinking level\n\n`/compact` — compact context\n\n`/cwd <path>` — set working directory\n\n`/help` — this help");
            }
            _ => {
                self.process_prompt(text, webhook.clone(), conversation_key)
                    .await?;
            }
        }
        Ok(())
    }

    /// Return this conversation's Agent session, or create it on first use.
    async fn get_or_create_session(&self, conversation_key: &str) -> Result<String> {
        if let Some(sid) = self.session_ids.read().await.get(conversation_key).cloned() {
            return Ok(sid);
        }
        let mut agent = self.agent.write().await;
        // Two first messages for one chat can race past the read above. The
        // Agent write lock serializes session creation, so recheck after it.
        if let Some(sid) = self.session_ids.read().await.get(conversation_key).cloned() {
            return Ok(sid);
        }
        let sid = agent.new_session(&self.agent_cfg.cwd, "dingtalk").await?;
        // Apply channel defaults once when this conversation is first seen.
        if !self.agent_cfg.model.is_empty() {
            let _ = agent.set_model(&sid, &self.agent_cfg.model).await;
        }
        if !self.agent_cfg.thinking_level.is_empty() {
            let _ = agent
                .set_thinking_level(&sid, &self.agent_cfg.thinking_level)
                .await;
        }
        if !self.agent_cfg.permission_level.is_empty() {
            let _ = agent
                .set_permission_level(&sid, &self.agent_cfg.permission_level)
                .await;
        }
        self.session_ids
            .write()
            .await
            .insert(conversation_key.to_string(), sid.clone());
        Ok(sid)
    }

    async fn process_prompt(
        &self,
        text: &str,
        webhook: Option<String>,
        conversation_key: &str,
    ) -> Result<()> {
        // Ordinary messages share one Agent session, so the Agent scheduler's
        // supersede_session policy can atomically replace the prior run. /new
        // is the only path that deliberately rotates this cached session.
        let session_id = self.get_or_create_session(conversation_key).await?;
        let agent = self.agent.clone();
        let dingtalk = self.dingtalk.clone();
        let text = text.to_string();
        let gen_counter = {
            let mut counters = self.gen_counters.write().await;
            counters
                .entry(conversation_key.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        tokio::spawn(async move {
            if let Err(e) =
                run_prompt_loop(&dingtalk, &agent, &session_id, &text, &gen_counter, webhook).await
            {
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
    let (expected_run_id, my_gen, mut stream) = {
        let mut client = agent.write().await;
        let send_preview = if text.len() > 300 {
            truncate_at_char(text, 300)
        } else {
            text.to_string()
        };
        info!(
            "[DING SEND] session={} text=\"{}\"",
            session_id, send_preview
        );
        let expected_run_id = client.prompt_superseding(session_id, text, vec![]).await?;
        client
            .wait_until_run_active(
                session_id,
                &expected_run_id,
                std::time::Duration::from_secs(30),
            )
            .await?;
        let stream = client
            .stream_run_events(session_id, &expected_run_id)
            .await?;
        let my_gen = gen_counter.fetch_add(1, Ordering::SeqCst) + 1;
        (expected_run_id, my_gen, stream)
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
        let parsed = match AgentClient::parse_event(event) {
            // Only consume events for the run we prompted; drop a different run's
            // events so a foreign agent_end can't finalize this reply.
            Some((rid, ev)) if rid.is_empty() || rid == expected_run_id => Some(ev),
            Some(_) => continue,
            None => None,
        };
        match parsed {
            Some(AgentEvent::AgentStart) | Some(AgentEvent::Ping) => {}
            Some(AgentEvent::ThinkingStart) => {
                stream_text.push_str("\n\n> 💭 **Thinking...**\n> \n> ");
            }
            Some(AgentEvent::ThinkingDelta(t)) => {
                stream_text.push_str(&t.replace('\n', "\n> "));
            }
            Some(AgentEvent::ThinkingEnd) => {
                stream_text.push_str("\n\n---\n\n");
            }
            Some(AgentEvent::TextChunk(chunk)) => {
                stream_text.push_str(&chunk);
            }
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
            Some(AgentEvent::AgentEnd { error, state }) => {
                check_superseded!();
                let was_cancelled = state.as_deref() == Some("cancelled");
                if let Some(err) = error {
                    if !was_cancelled
                        && !err.contains("interrupted")
                        && !err.contains("Interrupted")
                    {
                        if let Some(ref wh) = webhook {
                            dingtalk
                                .reply_webhook_markdown(wh, "Error", &format!("**Error:** {}", err))
                                .await?;
                        }
                    }
                } else if !stream_text.trim().is_empty() {
                    if let Some(ref wh) = webhook {
                        let preview = if stream_text.len() > 20000 {
                            format!("{}...", truncate_at_char(&stream_text, 20000))
                        } else {
                            stream_text.clone()
                        };
                        dingtalk
                            .reply_webhook_markdown(wh, "Future OS", &preview)
                            .await?;
                    }
                }
                break;
            }
            Some(AgentEvent::Error(err)) => {
                check_superseded!();
                if let Some(ref wh) = webhook {
                    dingtalk
                        .reply_webhook_markdown(wh, "Error", &format!("**Error:** {}", err))
                        .await?;
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

#[cfg(test)]
mod tests {
    // MockState scaffolding mutates Default::default() instances per-test by
    // design; field-reassign is the readable form for a 15-field mock.
    #![allow(clippy::field_reassign_with_default)]
    use super::*;
    use crate::config::AgentConfig;
    use crate::test_support::{self as ts, HttpRoute, MockState};

    const TOKEN_ROUTE: &str = "/v1.0/oauth2/accessToken";

    struct Fixture {
        bridge: DingtalkBridge,
        grpc: ts::SharedState,
        http: ts::RecordedRequests,
        /// Base URL of the mock HTTP server (for webhook URLs).
        base: String,
    }

    /// Bridge over mock gRPC + mock DingTalk REST (token route + webhook).
    async fn make_bridge(label: &str, state: MockState, extra_routes: Vec<HttpRoute>) -> Fixture {
        ts::ensure_crypto_provider();
        let _ = label;
        let mut routes = vec![
            HttpRoute::json(
                TOKEN_ROUTE,
                200,
                r#"{"accessToken":"dt-tok","expireIn":7200}"#,
            ),
            HttpRoute::json("/robot/hook", 200, "{}"),
        ];
        routes.extend(extra_routes);
        let (base, http) = ts::spawn_http(routes).await;
        let (addr, grpc) = ts::spawn_mock_grpc(state).await;
        let cfg = crate::dingtalk::config::DingtalkConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            domain: base.clone(), // full URL → base_url verbatim
        };
        let agent_cfg = Arc::new(AgentConfig {
            grpc_addr: addr,
            cwd: "/tmp".into(),
            model: "future/k3".into(),
            thinking_level: "high".into(),
            permission_level: "all".into(),
        });
        let bridge = DingtalkBridge::new(agent_cfg, cfg)
            .await
            .expect("bridge builds over mocks");
        Fixture {
            bridge,
            grpc,
            http,
            base,
        }
    }

    fn done_events() -> Vec<future_rpc::proto::StreamEvent> {
        vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"ding answer"}"#),
            ts::ev("", 1, "agent_end", r#"{"state":"completed"}"#),
        ]
    }

    fn event(base: &str, msg_id: &str, text: &str) -> DingtalkEvent {
        DingtalkEvent {
            event_type: "CALLBACK".into(),
            message_id: Some(msg_id.into()),
            chat_id: Some("cid-1".into()),
            chat_type: Some("1".into()),
            sender_id: Some("user-1".into()),
            sender_name: Some("Alice".into()),
            msg_type: Some("text".into()),
            content: Some(text.into()),
            create_time_ms: None,
            session_webhook: Some(format!("{}/robot/hook", base)),
            chatbot_user_id: Some("bot-1".into()),
            raw: serde_json::json!({}),
        }
    }

    fn hook_bodies(http: &ts::RecordedRequests) -> Vec<String> {
        ts::requests_to(http, "/robot/hook")
            .iter()
            .map(|r| r.body_string())
            .collect()
    }

    /// Wait until a webhook reply lands containing `needle`.
    async fn wait_hook(http: &ts::RecordedRequests, needle: &str) -> bool {
        ts::wait_until(
            || hook_bodies(http).iter().any(|b| b.contains(needle)),
            std::time::Duration::from_secs(10),
        )
        .await
    }

    // ─── Early-skip arms ─────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn skips_events_with_missing_fields_or_bot_self() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-skip", state, vec![]).await;
        // No sender.
        let mut e = event(&fx.base, "m1", "hi");
        e.sender_id = None;
        fx.bridge.handle_event(e).await.unwrap();
        // No message_id.
        let mut e = event(&fx.base, "m2", "hi");
        e.message_id = None;
        fx.bridge.handle_event(e).await.unwrap();
        // Bot's own message.
        let mut e = event(&fx.base, "m3", "hi");
        e.sender_id = Some("bot-1".into());
        fx.bridge.handle_event(e).await.unwrap();
        assert!(ts::recorded_of(&fx.grpc, "prompt").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dedup_and_stale_skips() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-dedup", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "hello"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "ding answer").await);
        // Redelivery → skipped.
        fx.bridge
            .handle_event(event(&fx.base, "m1", "hello"))
            .await
            .unwrap();
        assert_eq!(ts::recorded_of(&fx.grpc, "prompt").len(), 1);

        // Stale create_time → skipped (and the dedup set stays bounded).
        let old_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            - 120_000;
        for i in 0..1005 {
            let mut e = event(&fx.base, &format!("stale-{i}"), "old");
            e.create_time_ms = Some(old_ms);
            fx.bridge.handle_event(e).await.unwrap();
        }
        assert_eq!(ts::recorded_of(&fx.grpc, "prompt").len(), 1);
        let len = fx.bridge.processed.read().await.len();
        assert!(len <= 520, "dedup set trimmed, got {len}");
    }

    // ─── Prompt flow ─────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_streams_markdown_reply_via_webhook() {
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "thinking_start", "{}"),
            ts::ev("", 1, "thinking_delta", r#"{"text":"hmm\nso"}"#),
            ts::ev("", 2, "thinking_end", "{}"),
            ts::ev(
                "",
                3,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"shell"}"#,
            ),
            ts::ev(
                "",
                4,
                "tool_end",
                r#"{"tool_id":"t1","text":"line1\nline2"}"#,
            ),
            ts::ev("", 5, "tool_end", r#"{"tool_id":"t2"}"#),
            ts::ev("", 6, "text_chunk", r#"{"text":"the answer"}"#),
            ts::ev("", 7, "agent_end", r#"{"state":"completed"}"#),
        ];
        let fx = make_bridge("dt-prompt", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "question"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "the answer").await);
        let hooks = hook_bodies(&fx.http);
        let body = hooks.last().unwrap();
        assert!(body.contains("Thinking"), "{body}");
        assert!(body.contains("🔧"), "{body}");
        // Channel defaults applied at session creation.
        assert!(!ts::recorded_of(&fx.grpc, "set_model").is_empty());
        assert!(!ts::recorded_of(&fx.grpc, "set_thinking_level").is_empty());
        assert!(!ts::recorded_of(&fx.grpc, "set_permission_level").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_without_webhook_runs_but_sends_nothing() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-no-hook", state, vec![]).await;
        let mut e = event(&fx.base, "m1", "hello");
        e.session_webhook = None;
        fx.bridge.handle_event(e).await.unwrap();
        assert!(
            ts::wait_until(
                || !ts::recorded_of(&fx.grpc, "prompt").is_empty(),
                std::time::Duration::from_secs(5)
            )
            .await
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(hook_bodies(&fx.http).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_end_error_variants() {
        // error → webhook error reply.
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"partial"}"#),
            ts::ev("", 1, "agent_end", r#"{"error":"boom"}"#),
        ];
        let fx = make_bridge("dt-err", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "x"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "boom").await);

        // cancelled-with-error / interrupted → silent (the was_cancelled and
        // "interrupted" guards only apply when an error is present).
        for (label, data) in [
            ("dt-cancel", r#"{"state":"cancelled","error":"aborted"}"#),
            ("dt-interrupted", r#"{"error":"Interrupted by newer"}"#),
        ] {
            let mut state = MockState::default();
            state.events = vec![
                ts::ev("", 0, "text_chunk", r#"{"text":"partial"}"#),
                ts::ev("", 1, "agent_end", data),
            ];
            let fx = make_bridge(label, state, vec![]).await;
            fx.bridge
                .handle_event(event(&fx.base, "m1", "x"))
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            assert!(
                hook_bodies(&fx.http).is_empty(),
                "{label}: no reply expected for {data}"
            );
        }

        // error event → webhook error reply.
        let mut state = MockState::default();
        state.events = vec![ts::ev("", 0, "error", r#"{"error":"stream died"}"#)];
        let fx = make_bridge("dt-err-event", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "x"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "stream died").await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn long_reply_truncated_at_20000() {
        let big = "y".repeat(21000);
        let mut state = MockState::default();
        state.events = vec![
            ts::ev(
                "",
                0,
                "text_chunk",
                &serde_json::json!({"text": big}).to_string(),
            ),
            ts::ev("", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let fx = make_bridge("dt-long", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "x"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "yyy").await);
        let body = hook_bodies(&fx.http).pop().unwrap();
        assert!(body.len() < 21000 + 500, "reply must be truncated");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn supersede_and_foreign_run_arms() {
        // Foreign run events dropped entirely.
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("other", 0, "text_chunk", r#"{"text":"alien"}"#),
            ts::ev("other", 1, "agent_end", r#"{"state":"completed"}"#),
        ];
        let fx = make_bridge("dt-foreign", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "x"))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(hook_bodies(&fx.http).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_failure_is_logged_not_raised() {
        let mut state = MockState::default();
        state.fail_commands.insert("prompt".into());
        let fx = make_bridge("dt-prompt-fail", state, vec![]).await;
        // process_prompt spawns the loop; the failure is logged, handle_event
        // itself returns Ok.
        fx.bridge
            .handle_event(event(&fx.base, "m1", "x"))
            .await
            .unwrap();
        assert!(
            ts::wait_until(
                || !ts::recorded_of(&fx.grpc, "prompt").is_empty(),
                std::time::Duration::from_secs(5)
            )
            .await
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(hook_bodies(&fx.http).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_creation_race_uses_recheck() {
        // Slow new_session: two concurrent first-prompts for one conversation
        // — the loser of the write-lock race must reuse the winner's session
        // via the inner recheck (not create a second one).
        let mut state = MockState::default();
        state.events = done_events();
        state
            .command_delay
            .insert("new_session".into(), std::time::Duration::from_millis(300));
        let fx = make_bridge("dt-race", state, vec![]);
        let fx = fx.await;
        let b = &fx.bridge;
        let (r1, r2) = tokio::join!(
            b.get_or_create_session("cid-race"),
            b.get_or_create_session("cid-race")
        );
        assert_eq!(r1.unwrap(), r2.unwrap());
        assert_eq!(ts::recorded_of(&fx.grpc, "new_session").len(), 1);
    }

    // ─── Slash commands ──────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_without_webhook_returns_early() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-slash-nohook", state, vec![]).await;
        let mut e = event(&fx.base, "m1", "/status");
        e.session_webhook = None;
        fx.bridge.handle_event(e).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(ts::recorded_of(&fx.grpc, "get_state").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_new_session_lifecycle() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-slash-new", state, vec![]).await;
        // /new with no prior session → creates one.
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/new"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "New Session").await);
        assert_eq!(ts::recorded_of(&fx.grpc, "new_session").len(), 1);
        assert!(ts::recorded_of(&fx.grpc, "abort").is_empty());

        // /new again → aborts the old session, creates a fresh one.
        fx.bridge
            .handle_event(event(&fx.base, "m2", "/new"))
            .await
            .unwrap();
        assert_eq!(ts::recorded_of(&fx.grpc, "new_session").len(), 2);
        assert_eq!(ts::recorded_of(&fx.grpc, "abort").len(), 1);
        drop(fx);

        // new_session failure → error reply.
        let mut state = MockState::default();
        state.fail_commands.insert("new_session".into());
        let fx = make_bridge("dt-slash-new-fail", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/new"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Error").await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_status_reports_state() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-slash-status", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/status"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Model:").await);
        let body = hook_bodies(&fx.http).pop().unwrap();
        assert!(body.contains("future/k3"), "{body}");
        assert!(body.contains("Provider"), "{body}"); // model info block
        assert!(body.contains("Cost"), "{body}");
        drop(fx);

        // get_state failure → silent (no reply).
        let mut state = MockState::default();
        state.fail_commands.insert("get_state".into());
        // new_session must still work for session creation; get_state failing
        // means the status body never builds.
        let fx = make_bridge("dt-status-fail", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/status"))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(hook_bodies(&fx.http).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_stop_model_models_compact_effort_cwd() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-slash-misc", state, vec![]).await;

        fx.bridge
            .handle_event(event(&fx.base, "m1", "/stop"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Stopped").await);

        fx.bridge
            .handle_event(event(&fx.base, "m2", "/model future:plain"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Model:").await);
        assert!(ts::recorded_of(&fx.grpc, "set_model")
            .iter()
            .any(|c| c.model_id == "future/plain"));

        fx.bridge
            .handle_event(event(&fx.base, "m3", "/models"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "K3").await);

        fx.bridge
            .handle_event(event(&fx.base, "m4", "/compact"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Context compacted").await);

        fx.bridge
            .handle_event(event(&fx.base, "m5", "/effort turbo"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Invalid").await);
        fx.bridge
            .handle_event(event(&fx.base, "m6", "/effort high"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Thinking").await);

        fx.bridge
            .handle_event(event(&fx.base, "m7", "/cwd /work"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "CWD").await);
        assert!(ts::recorded_of(&fx.grpc, "set_cwd")
            .iter()
            .any(|c| c.cwd == "/work"));

        fx.bridge
            .handle_event(event(&fx.base, "m8", "/help"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Commands").await);

        // Unknown slash → forwarded to the agent.
        fx.bridge
            .handle_event(event(&fx.base, "m9", "/dance"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "ding answer").await);
        assert!(ts::recorded_of(&fx.grpc, "prompt")
            .iter()
            .any(|c| c.message.contains("/dance")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_silent_failure_arms() {
        // set_model/list_models/compact/set_thinking_level/set_cwd failures
        // are all silent (if-let-Ok skips the reply).
        let mut state = MockState::default();
        state.events = done_events();
        for cmd in [
            "set_model",
            "list_models",
            "compact",
            "set_thinking_level",
            "set_cwd",
        ] {
            state.fail_commands.insert(cmd.into());
        }
        let fx = make_bridge("dt-slash-fail", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/model x/y"))
            .await
            .unwrap();
        fx.bridge
            .handle_event(event(&fx.base, "m2", "/models"))
            .await
            .unwrap();
        fx.bridge
            .handle_event(event(&fx.base, "m3", "/compact"))
            .await
            .unwrap();
        fx.bridge
            .handle_event(event(&fx.base, "m4", "/effort low"))
            .await
            .unwrap();
        fx.bridge
            .handle_event(event(&fx.base, "m5", "/cwd /x"))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // Session creation itself triggered a set_model failure — tolerated.
        // None of the commands replied.
        assert!(hook_bodies(&fx.http).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_status_models_empty_variant() {
        // list_models returns nothing matching → empty model-info block.
        let mut state = MockState::default();
        state.events = done_events();
        state
            .responses
            .insert("list_models".into(), r#"{"models":[]}"#.into());
        let fx = make_bridge("dt-status-nomodels", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/status"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Model:").await);
        let body = hook_bodies(&fx.http).pop().unwrap();
        assert!(!body.contains("Provider"), "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_models_zero_context_window() {
        // contextWindow 0 → the ctx column is omitted.
        let mut state = MockState::default();
        state.responses.insert(
            "list_models".into(),
            r#"{"models":[{"id":"m0","label":"Zero","provider":"p","supportsImages":false,"contextWindow":0}]}"#.into(),
        );
        let fx = make_bridge("dt-models-zero", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/models"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Zero").await);
        let body = hook_bodies(&fx.http).pop().unwrap();
        assert!(!body.contains("ctx"), "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_loop_directly_with_long_text_and_empty_result() {
        // Direct call (same thread) so the thread-local subscriber governs
        // the send-log truncation arm.
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        // Long text (>300) → truncated send-log arm; empty stream_text at
        // agent_end → no-reply false path.
        let mut state = MockState::default();
        state.events = vec![ts::ev("", 0, "agent_end", r#"{"state":"completed"}"#)];
        let fx = make_bridge("dt-direct", state, vec![]).await;
        let sid = fx.bridge.get_or_create_session("cid-direct").await.unwrap();
        let agent = fx.bridge.agent.clone();
        let gen = AtomicU64::new(0);
        let long = "w".repeat(400);
        run_prompt_loop(&fx.bridge.dingtalk, &agent, &sid, &long, &gen, None)
            .await
            .unwrap();
        assert!(hook_bodies(&fx.http).is_empty(), "no text → no reply");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_fails_without_agent() {
        ts::ensure_crypto_provider();
        let cfg = crate::dingtalk::config::DingtalkConfig {
            client_id: "id".into(),
            client_secret: "s".into(),
            domain: "api.dingtalk.com".into(),
        };
        let agent_cfg = Arc::new(AgentConfig {
            grpc_addr: "127.0.0.1:1".into(),
            cwd: "/tmp".into(),
            model: String::new(),
            thinking_level: String::new(),
            permission_level: String::new(),
        });
        assert!(DingtalkBridge::new(agent_cfg, cfg).await.is_err());
    }

    // ─── truncate helpers ────────────────────────────────────────────────────

    #[test]
    fn truncate_at_char_boundaries() {
        assert_eq!(truncate_at_char("hello", 10), "hello");
        assert_eq!(truncate_at_char("hello world", 5), "hello");
        assert_eq!(truncate_at_char("你好世界", 2), "你好");
        assert_eq!(truncate_at_char("", 5), "");
    }

    #[test]
    fn truncate_tool_output_rules() {
        // Short output unchanged.
        assert_eq!(truncate_tool_output("a\nb"), "a\nb");
        // Line-limit truncation.
        let many_lines = (1..=10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = truncate_tool_output(&many_lines);
        assert!(out.contains("line4"));
        assert!(!out.contains("line6"));
        assert!(out.contains("truncated"));
        // Char-limit truncation.
        let long_line = "x".repeat(600);
        let out = truncate_tool_output(&long_line);
        assert!(out.len() < 600);
        assert!(out.contains("truncated"));
        // Unicode-safe.
        let uni = "好".repeat(600);
        let out = truncate_tool_output(&uni);
        assert!(out.contains("truncated"));
    }

    // ─── Residual-arm chase ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn event_without_chatbot_id_and_fresh_timestamp() {
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-nobotid", state, vec![]).await;
        let mut e = event(&fx.base, "m1", "hello");
        e.chatbot_user_id = None; // skip the bot-self check entirely
        e.create_time_ms = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        ); // fresh → stale-check false arm
        fx.bridge.handle_event(e).await.unwrap();
        assert!(wait_hook(&fx.http, "ding answer").await);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_text_log_arms_with_subscriber() {
        // current_thread: the thread-local subscriber governs log-arg
        // evaluation (multi_thread migrates tasks off the subscribed thread).
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-longtext", state, vec![]).await;
        let long = "z".repeat(400); // >200 recv log arm, >300 send log arm
        fx.bridge
            .handle_event(event(&fx.base, "m1", &long))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "ding answer").await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_session_create_failure_replies_error() {
        let mut state = MockState::default();
        state.fail_commands.insert("new_session".into());
        let fx = make_bridge("dt-sess-fail", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/status"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "Error").await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_model_without_arg_is_silent() {
        // "/model" with no arg: matched by the outer group arm, falls through
        // the inner guarded arms to `_ => {}` — no reply.
        let mut state = MockState::default();
        state.events = done_events();
        let fx = make_bridge("dt-model-noarg", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "/model"))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(hook_bodies(&fx.http).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_channel_defaults_skip_set_calls() {
        ts::ensure_crypto_provider();
        let mut state = MockState::default();
        state.events = done_events();
        let (base, http) = ts::spawn_http(vec![
            HttpRoute::json(
                TOKEN_ROUTE,
                200,
                r#"{"accessToken":"dt-tok","expireIn":7200}"#,
            ),
            HttpRoute::json("/robot/hook", 200, "{}"),
        ])
        .await;
        let (addr, grpc) = ts::spawn_mock_grpc(state).await;
        let cfg = crate::dingtalk::config::DingtalkConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            domain: base.clone(),
        };
        let agent_cfg = Arc::new(AgentConfig {
            grpc_addr: addr,
            cwd: "/tmp".into(),
            model: String::new(),
            thinking_level: String::new(),
            permission_level: String::new(),
        });
        let bridge = DingtalkBridge::new(agent_cfg, cfg).await.unwrap();
        bridge
            .handle_event(event(&base, "m1", "hello"))
            .await
            .unwrap();
        assert!(wait_hook(&http, "ding answer").await);
        assert!(ts::recorded_of(&grpc, "set_model").is_empty());
        assert!(ts::recorded_of(&grpc, "set_thinking_level").is_empty());
        assert!(ts::recorded_of(&grpc, "set_permission_level").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loop_event_variants_and_no_webhook_error_arms() {
        // agent_start/ping (no-op), tool_delta (catch-all), unmappable event
        // (parse None), then a normal completion.
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "agent_start", "{}"),
            ts::ev("", 1, "ping", ""),
            ts::ev("", 2, "tool_delta", r#"{"tool_id":"t","text":"x"}"#),
            ts::ev("", 3, "session_info", "{}"),
            ts::ev("", 4, "text_chunk", r#"{"text":"body"}"#),
            ts::ev("", 5, "agent_end", r#"{"state":"completed"}"#),
        ];
        let fx = make_bridge("dt-variants", state, vec![]).await;
        fx.bridge
            .handle_event(event(&fx.base, "m1", "x"))
            .await
            .unwrap();
        assert!(wait_hook(&fx.http, "body").await);

        // agent_end error with NO webhook → skipped silently.
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("", 0, "text_chunk", r#"{"text":"p"}"#),
            ts::ev("", 1, "agent_end", r#"{"error":"quiet boom"}"#),
        ];
        let fx = make_bridge("dt-err-nohook", state, vec![]).await;
        let mut e = event(&fx.base, "m1", "x");
        e.session_webhook = None;
        fx.bridge.handle_event(e).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(hook_bodies(&fx.http).is_empty());

        // error event with NO webhook → also silent.
        let mut state = MockState::default();
        state.events = vec![ts::ev("", 0, "error", r#"{"error":"quiet"}"#)];
        let fx = make_bridge("dt-errevt-nohook", state, vec![]).await;
        let mut e = event(&fx.base, "m1", "x");
        e.session_webhook = None;
        fx.bridge.handle_event(e).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(hook_bodies(&fx.http).is_empty());
    }
}
