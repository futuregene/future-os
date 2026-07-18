use crate::events::EventBus;
use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use super::{ApprovalGate, SseBroadcaster};

// Reverted to "workspace": commit 49eab817 flipped this to "all" (no boundary
// enforcement at all) in an unrelated change and broke the test below.
const DEFAULT_PERMISSION_LEVEL: &str = "workspace";

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
    pub parent_session_id: String,
    pub session_name: String,
    /// Source that created this session: "gui", "tui", "fork", "feishu", "dingtalk", etc.
    pub created_by: String,
    /// Arbitrary metadata from the source side (JSON). Free-form.
    pub source_meta: serde_json::Value,
    pub event_bus: Arc<EventBus>,
    pub broadcaster: Arc<SseBroadcaster>,
    pub ephemeral: bool,
    /// Cumulative token counters (Arc<AtomicI64> — read lock-free without agent_loop lock)
    pub tokens_in: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_out: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_cache_r: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_cache_w: Arc<std::sync::atomic::AtomicI64>,
    /// Cumulative cost as reported by upstream (Future API `credit_cost`).
    pub cumulative_cost: Arc<std::sync::Mutex<f64>>,
    /// Last API call's prompt_tokens (actual context size, reset each call)
    pub last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Sender for steering queue (cloned from loop, usable without loop lock)
    pub steering_tx: tokio::sync::mpsc::Sender<String>,
    /// Sender for follow_up queue
    pub follow_up_tx: tokio::sync::mpsc::Sender<String>,
    /// Sender for interrupting the current stream (per-stream, set in prompt())
    pub interrupt_tx: Option<tokio::sync::mpsc::Sender<()>>,
    /// Interrupt flag cloned from the agent loop (Arc<AtomicBool>), settable
    /// **without** the agent_loop lock — which `abort()` cannot acquire while a
    /// run holds `agent_loop.write()`. This is what makes abort actually cancel
    /// in-flight tools (shell) and break between tool calls, not just the stream.
    /// Set alongside `interrupt_tx` in `prompt()`.
    pub interrupt_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub approval_gate: ApprovalGate,
    /// Permission level for tool execution: "all" | "workspace" | "none"
    pub permission_level: String,
    /// Sandbox + approval policy. `None` = the sandbox stays dormant and the
    /// session behaves exactly like the pre-sandbox agent (legacy boundary, no
    /// OS wrapping). Only a client that sends `set_sandbox_policy` opts in —
    /// today that's just the GUI, which owns the approval UX. TUI / CLI /
    /// channels never send one, so they are unaffected.
    pub sandbox_policy: Option<crate::sandbox::SandboxPolicy>,
    /// Runtime "allow in this workspace/chat" rules for the current run. Shared
    /// into the live sandbox at prompt start; cleared each new run.
    pub session_rules: crate::sandbox::rules::SessionRules,
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

/// Resolve the API key for a model, in priority order: an entry keyed by the
/// exact model id, then by its provider, then the model's own configured key
/// (when non-empty), then the account-wide default. Empty string when none match.
fn resolve_api_key(
    auth: &crate::AuthStore,
    model: &str,
    provider: &str,
    model_key: &str,
) -> String {
    auth.get(model)
        .or_else(|| auth.get(provider))
        .or_else(|| (!model_key.is_empty()).then(|| model_key.to_string()))
        .or_else(|| auth.default_key())
        .unwrap_or_default()
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
            thinking_level: "xhigh".to_string(), // Match default
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            auto_compaction: true, // Match default
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session_name: String::new(),
            parent_session_id: String::new(),
            created_by: String::new(),
            source_meta: serde_json::Value::Null,
            event_bus,
            broadcaster,
            ephemeral: false,
            tokens_in: ti,
            tokens_out: to,
            tokens_cache_r: tcr,
            tokens_cache_w: tcw,
            cumulative_cost: Arc::new(std::sync::Mutex::new(0.0)),
            last_prompt_tokens: lpt,
            steering_tx: stx,
            follow_up_tx: ftx,
            interrupt_tx: None,
            interrupt_flag: None,
            approval_gate,
            permission_level: DEFAULT_PERMISSION_LEVEL.to_string(),
            sandbox_policy: None,
            session_rules: std::sync::Arc::new(std::sync::Mutex::new(vec![])),
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
            thinking_level: "xhigh".to_string(),
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            auto_compaction: true,
            auto_retry: true,
            session_manager: manager,
            cwd: cwd.to_string(),
            is_streaming: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session_name: String::new(),
            parent_session_id: String::new(),
            created_by: String::new(),
            source_meta: serde_json::Value::Null,
            event_bus,
            broadcaster,
            ephemeral: false,
            tokens_in: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            tokens_out: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            tokens_cache_r: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            tokens_cache_w: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cost: Arc::new(std::sync::Mutex::new(0.0)),
            last_prompt_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            steering_tx: stx,
            follow_up_tx: ftx,
            interrupt_tx: None,
            interrupt_flag: None,
            approval_gate,
            permission_level: DEFAULT_PERMISSION_LEVEL.to_string(),
            sandbox_policy: None,
            session_rules: std::sync::Arc::new(std::sync::Mutex::new(vec![])),
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
            return self.prompt(msg, &[], &[]);
        }
        let _ = self.follow_up_tx.try_send(msg.to_string());
        Ok(())
    }

    pub fn abort(&self) {
        // Primary path: send via interrupt_tx for streaming-mode abort
        if let Some(ref tx) = self.interrupt_tx {
            let _ = tx.try_send(());
        }
        // Set the interrupt flag via the lock-free Arc clone captured at prompt
        // start. This is the load-bearing part: a run holds `agent_loop.write()`,
        // so the `try_read()` below fails mid-run and can't set the flag — without
        // this, abort never cancels an in-flight tool (e.g. a long shell command) or breaks
        // between tool calls; it only stops at the next LLM boundary.
        if let Some(ref flag) = self.interrupt_flag {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        // Fallback for a run that started before the flag was captured.
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
        // Store full provider/id as the canonical model identifier for display
        // and session persistence. Resolve bare ID to provider/id when possible.
        self.model = resolved
            .as_ref()
            .map(|m| format!("{}/{}", m.provider, m.id))
            .unwrap_or_else(|| model.to_string());
        // Keep compaction closure in sync so /model changes are reflected.
        *self.compaction_model.write().unwrap() = self.model.clone();

        // Update the agent loop in one shot — both model name and provider endpoint.
        // Fail explicitly when the loop is busy so the caller knows to retry
        // rather than silently continuing with the old model.
        let mut loop_ = self.agent_loop.try_write().map_err(|_| {
            anyhow::anyhow!("agent is currently streaming; retry /model before your next prompt")
        })?;
        // Set agent loop model to bare canonical ID for LLM API calls.
        // The session-level self.model already holds the full provider/id.
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
            let api_key =
                resolve_api_key(&auth, model, &model_config.provider, &model_config.api_key);

            // Build a FRESH provider (its own reqwest client) and swap it in,
            // rather than mutating the existing provider's endpoint. Sessions
            // are seeded from a shared provider `Arc` in `new_session`, so
            // mutating it in place would (a) serialize concurrent sessions onto
            // one HTTP connection and (b) let one session's endpoint change
            // clobber another's mid-run. A per-session client makes concurrent
            // conversations use independent connections. The GUI calls
            // `set_model` on every session before prompting, so this is where
            // each session gets its own client.
            let max_tokens = if model_config.max_tokens > 0 {
                Some(std::cmp::min(model_config.max_tokens, 32000))
            } else if model_config.reasoning {
                Some(32000)
            } else {
                Some(16384)
            };
            // maxTokensField: compat field controlling max_tokens vs max_completion_tokens
            let max_tokens_field = model_config
                .compat
                .get("maxTokensField")
                .and_then(|v| v.as_str())
                .unwrap_or("max_tokens")
                .to_string();

            let mut client =
                crate::llm::Client::new(&model_config.base_url, &api_key, None, max_tokens)
                    .with_compat(
                        &thinking_format,
                        supports_reasoning_effort,
                        requires_reasoning_on_assistant,
                    )
                    .with_max_tokens_field(&max_tokens_field)
                    .with_thinking_level_map(tlm);
            // Carry the session's current thinking level/budget onto the new
            // client; an explicit set_thinking_level afterward still overrides.
            if !self.thinking_level.is_empty() {
                client = client.with_thinking_level(&self.thinking_level);
            }
            let thinking_budget = loop_.config.thinking_budget;
            if thinking_budget > 0 {
                client = client.with_thinking_budget(thinking_budget);
            }

            loop_.provider = std::sync::Arc::new(client);
        }
        Ok(())
    }

    /// Re-resolve the API key for this session's current model from disk
    /// (auth.json) and push it into the live provider. Called when credentials
    /// change out-of-band — FutureGene login/logout, custom-provider key edits —
    /// so the session doesn't keep serving prompts with the stale in-memory key
    /// until the next `set_model` (the prompt path never re-reads auth.json).
    ///
    /// Unlike `set_model` this stays correct even when the model no longer
    /// resolves: after logout the Future models drop out of the registry, so a
    /// `resolve` miss must NOT leave the old key in place. We derive the provider
    /// from the canonical `provider/id` model id and, resolving no key, clear the
    /// credential so the stale one can't keep being used. The key-resolution
    /// order mirrors `set_model` for parity.
    ///
    /// A session actively streaming holds the loop write lock; `try_read` then
    /// fails and we skip it — it picks up the refreshed key on its next
    /// `set_model` (mid-run the in-flight request has already sent its header).
    pub fn reload_credentials(&self) {
        if self.model.is_empty() {
            return;
        }
        let registry = crate::models::Registry::new();
        let resolved = registry.resolve(&self.model);
        let provider = resolved
            .as_ref()
            .map(|m| m.provider.clone())
            .unwrap_or_else(|| self.model.split('/').next().unwrap_or("").to_string());

        let auth = crate::AuthStore::load();
        let model_key = resolved
            .as_ref()
            .map(|m| m.api_key.clone())
            .unwrap_or_default();
        let api_key = resolve_api_key(&auth, &self.model, &provider, &model_key);

        if let Ok(loop_) = self.agent_loop.try_read() {
            loop_.provider.set_api_key(&api_key);
        }
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
            .unwrap_or(1_000_000); // Modern default: 1M

        let reserve_tokens = ((context_window as f64 * 0.1) as i32).max(16384);
        let keep_tokens = ((context_window as f64 * 0.2) as i32).max(reserve_tokens);
        let (compacted, result) = crate::compaction::compact(
            messages,
            &crate::compaction::CompactOptions {
                reserve_tokens,
                keep_recent_tokens: keep_tokens,
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

    pub fn execute_shell(&self, command: &str) -> Result<serde_json::Value> {
        // Same platform-shell contract as the shell tool (bash -c on Unix,
        // the PowerShell wrapper on Windows) so exit codes are reliable.
        let (program, args) = crate::sandbox::shell_invocation(command);
        let output = std::process::Command::new(program)
            .args(&args)
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
            let effective_model = if session.model.is_empty() {
                self.model.clone()
            } else {
                session.model.clone()
            };
            let supports_images = crate::models::model_accepts_images(&effective_model);
            let msgs = crate::session::entries_to_agent_messages(&session.entries, supports_images);
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
                if let Some(cost) = info.get("total_cost").and_then(|v| v.as_f64()) {
                    if let Ok(mut c) = self.cumulative_cost.lock() {
                        *c = cost;
                    }
                }
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

    pub fn set_sandbox_policy(&mut self, policy: crate::sandbox::SandboxPolicy) {
        self.sandbox_policy = Some(policy);
    }

    /// Inject a same-run "allow in this workspace/chat" rule (from the GUI, in
    /// tandem with writing the rule file). Takes effect for the live run's
    /// subsequent tool calls; the file carries it to future runs.
    pub fn add_session_rule(&self, raw_pattern: &str, access: &str) {
        crate::sandbox::rules::push_session_allow(
            &self.session_rules,
            std::path::Path::new(&self.cwd),
            raw_pattern,
            crate::sandbox::rules::Access::parse(access),
        );
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
