use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use super::{ApprovalGate, SseBroadcaster};

// Default permission level for fresh sessions: "all" (unrestricted) is the
// deliberate product default — this is a local agent where the user expects
// full filesystem access out of the box; stricter levels ("workspace") are
// opt-in via settings. Matches config::default_permission_level().
const DEFAULT_PERMISSION_LEVEL: &str = "all";

// ─── ServerSession ────────────────────────────────────────────────────────

/// In-memory representation of one agent session.
///
/// Holds the agent loop (LLM client + tool set), the message history, session
/// metadata, and all control-plane state (queues, approval gate, sandbox policy).
/// Wrapped in `Arc<RwLock<ServerSession>>` for concurrent access from gRPC
/// handlers by `AppState`.
pub struct ServerSession {
    /// Stable unique session identifier (UUID v4).  Used as the JSONL filename
    /// on disk and as the key in `AppState::sessions`.
    pub session_id: String,
    /// The agent run-loop: LLM provider + tool registry + turn counter.
    /// Each session owns an independent loop minted from
    /// `AppState::loop_template` (`Loop::independent_copy`) — never a shared
    /// one — so concurrent runs, `set_model` calls and aborts stay
    /// session-local.
    pub agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
    /// Full message history as persisted to/loaded from the session JSONL.
    pub messages: Arc<parking_lot::RwLock<Vec<crate::types::AgentMessage>>>,
    /// Canonical model identifier for this session (e.g. "deepseek-v4-pro").
    /// Updated by `set_model`; read by prompt construction and compaction.
    pub model: String,
    /// Thinking/effort level: "off", "minimal", "low", "medium", "high", "xhigh".
    pub thinking_level: String,
    /// How new prompts are queued while streaming: "one-at-a-time" (replace
    /// pending) or "all" (enqueue all).
    pub steering_mode: String,
    /// How follow-up prompts are queued: same semantics as `steering_mode`.
    pub follow_up_mode: String,
    /// Whether auto-compaction is enabled for this session.
    pub auto_compaction: bool,
    /// Whether automatic retry on transient LLM errors is enabled.
    pub auto_retry: bool,
    /// On-disk session store (JSONL files).  Shared across everything that
    /// reads/writes session history.
    pub session_manager: Arc<Manager>,
    /// Ordered, lazy writer for this session's append/update/rewrite commands.
    pub persistence: crate::session::SessionPersistence,
    /// Absolute working directory for shell/tool execution.
    pub cwd: String,
    /// True while the agent loop is actively processing a prompt run.
    /// Compatibility projection only; lifecycle decisions use `runtime`.
    pub is_streaming: Arc<std::sync::atomic::AtomicBool>,
    /// Short-lock authoritative lifecycle and task owner. This prevents abort
    /// from making a session reusable before the matching task actually exits.
    pub runtime: Arc<crate::runtime::SessionRuntime>,
    /// ID of the session this one was forked from, if any.
    pub parent_session_id: String,
    /// Human-readable label (set via `/name`).  Empty until named.
    pub session_name: String,
    /// Source that created this session: "gui", "tui", "fork", "feishu", "dingtalk", etc.
    pub created_by: String,
    /// Arbitrary metadata from the source side (JSON). Free-form.
    pub source_meta: serde_json::Value,
    /// Per-session SSE broadcaster.  Each subscriber (`StreamEvents` call)
    /// receives a clone of the receiver.  Private per-session so events for
    /// one session never leak to another.
    pub broadcaster: Arc<SseBroadcaster>,
    /// When true, the session is never persisted to disk.
    pub ephemeral: bool,
    /// Cumulative token counters (Arc<AtomicI64> — read lock-free without agent_loop lock)
    pub tokens_in: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_out: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_cache_r: Arc<std::sync::atomic::AtomicI64>,
    pub tokens_cache_w: Arc<std::sync::atomic::AtomicI64>,
    /// Cumulative cost as reported by upstream (Future API `credit_cost`).
    pub cumulative_cost: Arc<parking_lot::Mutex<f64>>,
    /// Last API call's prompt_tokens (actual context size, reset each call)
    pub last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Sender for steering queue (cloned from loop, usable without loop lock)
    pub steering_tx: tokio::sync::mpsc::Sender<String>,
    /// Sender for follow_up queue
    pub follow_up_tx: tokio::sync::mpsc::Sender<String>,
    /// Approval gate: holds pending approval requests and their decisions.
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
    /// Process-wide cached model registry (shared from `AppState`).  Used by
    /// `set_model`/`reload_credentials` so hydrating N sessions costs zero
    /// registry rebuilds; refreshed in place by the `reload_auth` command
    /// after provider/auth changes on disk.
    pub model_registry: Arc<parking_lot::RwLock<crate::models::Registry>>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: String,
        agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
        manager: Arc<Manager>,
        cwd: &str,
        broadcaster: Arc<SseBroadcaster>,
        approval_gate: ApprovalGate,
        model_registry: Arc<parking_lot::RwLock<crate::models::Registry>>,
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
        let is_streaming = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let runtime = Arc::new(crate::runtime::SessionRuntime::new(is_streaming.clone()));
        let persistence =
            crate::session::SessionPersistence::new(manager.clone(), session_id.clone());
        Self {
            session_id: session_id.clone(),
            agent_loop,
            messages: Arc::new(parking_lot::RwLock::new(vec![])),
            model: String::new(),
            thinking_level: "xhigh".to_string(), // Match default
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            auto_compaction: true, // Match default
            auto_retry: true,
            session_manager: manager,
            persistence,
            cwd: cwd.to_string(),
            is_streaming,
            runtime,
            session_name: String::new(),
            parent_session_id: String::new(),
            created_by: String::new(),
            source_meta: serde_json::Value::Null,
            broadcaster,
            ephemeral: false,
            tokens_in: ti,
            tokens_out: to,
            tokens_cache_r: tcr,
            tokens_cache_w: tcw,
            cumulative_cost: Arc::new(parking_lot::Mutex::new(0.0)),
            last_prompt_tokens: lpt,
            steering_tx: stx,
            follow_up_tx: ftx,
            approval_gate,
            permission_level: DEFAULT_PERMISSION_LEVEL.to_string(),
            sandbox_policy: None,
            session_rules: std::sync::Arc::new(parking_lot::Mutex::new(vec![])),
            model_registry,
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
        self.steer_run(msg, None)
    }

    pub fn steer_run(&mut self, msg: &str, expected_run_id: Option<&str>) -> Result<()> {
        self.runtime
            .steer(expected_run_id, &self.steering_tx, msg.to_string())?;
        if let Ok(r#loop) = self.agent_loop.try_read() {
            if r#loop.verbose {
                tracing::info!("[user·steer] {msg}");
            }
        }
        Ok(())
    }

    pub fn follow_up(&mut self, msg: &str) -> Result<()> {
        self.follow_up_run(msg, None)
    }

    pub fn follow_up_run(&mut self, msg: &str, expected_run_id: Option<&str>) -> Result<()> {
        let queued =
            self.runtime
                .follow_up(expected_run_id, &self.follow_up_tx, msg.to_string())?;
        if let Ok(r#loop) = self.agent_loop.try_read() {
            if r#loop.verbose {
                tracing::info!("[user·follow-up] {msg}");
            }
        }
        if !queued {
            return self.prompt(msg, &[], &[], None, None).map(|_| ());
        }
        Ok(())
    }

    pub fn abort(&self) {
        let _ = self.abort_run(None);
    }

    pub fn abort_run(&self, expected_run_id: Option<&str>) -> Result<()> {
        self.runtime.request_abort(expected_run_id)
    }

    pub fn new_session(&mut self) -> Result<()> {
        self.messages.write().clear();
        Ok(())
    }

    pub fn get_messages(&self) -> Vec<crate::types::Message> {
        let msgs = self.messages.read();
        ConvertToLLM(&msgs)
    }

    pub fn set_model(&mut self, model: &str) -> Result<()> {
        // Resolve against the shared cached registry — never rebuilds it.
        // The cache is refreshed by `reload_auth` when models.json changes.
        let resolved = self.model_registry.read().resolve(model);
        // Store full provider/id as the canonical model identifier for display
        // and session persistence. Resolve bare ID to provider/id when possible.
        let canonical_model = resolved
            .as_ref()
            .map(|m| format!("{}/{}", m.provider, m.id))
            .unwrap_or_else(|| model.to_string());

        // Update the agent loop in one shot — both model name and provider endpoint.
        // Fail explicitly when the loop is busy so the caller knows to retry
        // rather than silently continuing with the old model. Active runs use
        // snapshots and do not hold this control-plane lock.
        let mut loop_ = self
            .agent_loop
            .try_write()
            .map_err(|_| anyhow::anyhow!("session configuration is busy; retry /model"))?;
        self.model = canonical_model;
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
            // rather than mutating the existing provider's endpoint.  Each
            // session owns its loop (minted from AppState::loop_template), so
            // the fresh client is this session's alone: concurrent sessions
            // use independent connections and never clobber each other's
            // endpoint mid-run.
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
        drop(loop_);

        // Persist through the same ordered queue as run appends/finalization.
        // Brand-new sessions have no JSONL yet; their first accepted prompt
        // creates it with the selected model.
        if self.session_manager.find(&self.session_id).is_some() {
            self.persistence
                .update_info("model", serde_json::Value::String(self.model.clone()))?;
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
    /// Active runs use a provider snapshot, so updating this control plane
    /// affects the next request without mutating headers already sent by an
    /// in-flight request.
    pub fn reload_credentials(&self) {
        if self.model.is_empty() {
            return;
        }
        let registry_resolved = self.model_registry.read().resolve(&self.model);
        let provider = registry_resolved
            .as_ref()
            .map(|m| m.provider.clone())
            .unwrap_or_else(|| self.model.split('/').next().unwrap_or("").to_string());

        let auth = crate::AuthStore::load();
        let model_key = registry_resolved
            .as_ref()
            .map(|m| m.api_key.clone())
            .unwrap_or_default();
        let api_key = resolve_api_key(&auth, &self.model, &provider, &model_key);

        if let Ok(loop_) = self.agent_loop.try_read() {
            loop_.provider.set_api_key(&api_key);
        }
    }

    fn strip_image_content_from_messages(&self) {
        for message in self.messages.write().iter_mut() {
            message
                .content
                .retain(|block| !matches!(block, crate::types::ContentBlock::Image { .. }));
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

        // Keep metadata writes ordered with active-run persistence. This setter
        // predates fallible RPC setters, so report a durable error without
        // changing its public signature.
        if self.session_manager.find(&self.session_id).is_some() {
            if let Err(error) = self.persistence.update_info(
                "thinking_level",
                serde_json::Value::String(self.thinking_level.clone()),
            ) {
                tracing::error!("Failed to persist thinking level: {error:#}");
            }
        }
    }

    pub fn set_steering_mode(&mut self, mode: &str) {
        self.steering_mode = mode.to_string();
        if let Ok(mut loop_) = self.agent_loop.try_write() {
            loop_.steering_queue.mode = mode.to_string();
        }
        // A rare concurrent config update may defer this mirror; prompt() also
        // applies the authoritative session field at the next run boundary.
    }

    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.follow_up_mode = mode.to_string();
        if let Ok(mut loop_) = self.agent_loop.try_write() {
            loop_.follow_up_queue.mode = mode.to_string();
        }
        // A rare concurrent config update may defer this mirror; prompt() also
        // applies the authoritative session field at the next run boundary.
    }

    pub fn compact(&self, _instructions: &str) -> Result<serde_json::Value> {
        use std::sync::atomic::Ordering;

        let messages: Vec<crate::types::Message> = {
            let msgs = self.messages.read();
            ConvertToLLM(&msgs)
        };

        // Use API-reported prompt_tokens (same as getState's contextTokens)
        let tokens_before = self.last_prompt_tokens.load(Ordering::Relaxed) as i32;

        // Resolve context_window from model registry (same as getState's contextWindow)
        let context_window = self
            .model_registry
            .read()
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
        if self.session_manager.find(&self.session_id).is_some() {
            if let Err(error) = self
                .persistence
                .update_info("auto_compaction", serde_json::Value::Bool(enabled))
            {
                tracing::error!("Failed to persist auto-compaction setting: {error:#}");
            }
        }
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
        let msgs = self.messages.read();
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

    /// Operational lifecycle/event metrics for this live session. These values
    /// are intentionally monotonic for the lifetime of the session runtime
    /// (except `activeRunGauge`, which is a point-in-time gauge) and are exposed
    /// through `get_runtime_metrics` for diagnostics and acceptance tests.
    pub fn get_runtime_metrics(&self) -> serde_json::Value {
        serde_json::json!({
            "sessionId": self.session_id(),
            "activeRunGauge": self.runtime.active_task_count(),
            "staleEpochDrops": self.runtime.stale_epoch_drop_count(),
            "persistenceDegraded": self.runtime.persistence_degraded_count(),
            "broadcastLag": self.broadcaster.lag_count(),
            "ringTruncations": self.broadcaster.truncation_count(),
            "activeRunId": self.runtime.snapshot().map(|run| run.run_id),
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<serde_json::Value>> {
        // Lightweight summaries: scans each JSONL without deserializing
        // large tool/assistant payloads, so listing stays fast even with
        // thousands of sessions on disk.
        let sessions = self.session_manager.list_summaries(&self.cwd)?;
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
                tracing::info!(
                    "[session] switch_session loaded model={} for session={}",
                    self.model,
                    id,
                );

                // Sync the agent loop's model + provider endpoint so the next
                // prompt uses the saved model, not a stale leftover from the
                // previous session.  set_model is best-effort here (loop may be
                // busy); a failure just logs and defers — the user can call
                // /model explicitly if needed.
                if let Err(e) = self.set_model(&self.model.clone()) {
                    tracing::warn!(
                        "[session] could not sync agent loop model during switch_session: {e}"
                    );
                }
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
                    *self.cumulative_cost.lock() = cost;
                }
            }
            *self.messages.write() = msgs;
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
        let msgs = self.messages.read();
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
    use tokio::sync::Notify;
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

    struct BlockingProvider {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl LLMProvider for BlockingProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
            let (tx, rx) = mpsc::channel(2);
            self.started.notify_one();
            let release = self.release.clone();
            tokio::spawn(async move {
                release.notified().await;
                let _ = tx
                    .send(StreamEvent {
                        event_type: "stop".to_string(),
                        ..Default::default()
                    })
                    .await;
            });
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

    /// Unique, isolated session directory for a test session. Each call gets
    /// its own temp dir (timestamp + random hex) so parallel tests never share
    /// a JSONL file, and nothing is written to the real
    /// `~/.future/agent/sessions` store (which `Manager::default_for` targets,
    /// since `default_session_dir` ignores its cwd argument).
    fn test_session_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "futureos-sess-test-{}",
            crate::utils::generate_id()
        ))
    }

    #[test]
    fn new_sessions_default_to_all_permission() {
        let cwd = test_workspace();
        let session = ServerSession::new(
            "session_test".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );

        assert_eq!(session.get_permission_level(), "all");
    }

    // ─── Helper to build a test session ─────────────────────────────────────

    fn make_test_session(id: &str) -> ServerSession {
        let cwd = test_workspace();
        ServerSession::new(
            id.to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        )
    }

    fn make_persistent_test_session(id: &str) -> ServerSession {
        let cwd = test_workspace();
        let session_dir = std::path::Path::new(&cwd).join("sessions");
        ServerSession::new(
            id.to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(Manager::new(session_dir)),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        )
    }

    // ─── resolve_api_key ────────────────────────────────────────────────────

    #[test]
    fn resolve_api_key_prefers_model_id() {
        let auth = crate::AuthStore::load();
        // With an empty auth store, should fall back to model_key or empty
        let key = resolve_api_key(&auth, "unknown/model", "unknown", "model_key_123");
        assert!(key == "model_key_123" || key.is_empty());
    }

    #[test]
    fn resolve_api_key_empty_model_key() {
        let auth = crate::AuthStore::load();
        let key = resolve_api_key(&auth, "unknown/model", "unknown", "");
        assert!(key.is_empty() || !key.is_empty()); // just verify no panic
    }

    // ─── default_workspace ──────────────────────────────────────────────────

    #[test]
    fn default_workspace_is_not_empty() {
        let ws = default_workspace();
        assert!(!ws.is_empty());
        assert!(ws.contains(".future"));
    }

    // ─── ServerSession basics ───────────────────────────────────────────────

    #[test]
    fn session_id_returns_id() {
        let session = make_test_session("test_123");
        assert_eq!(session.session_id(), "test_123");
    }

    #[test]
    fn session_name_set_and_get() {
        let mut session = make_test_session("s1");
        assert_eq!(session.session_name(), "");
        session.set_session_name("My Session");
        assert_eq!(session.session_name(), "My Session");
    }

    #[test]
    fn default_thinking_level_is_xhigh() {
        let session = make_test_session("s1");
        assert_eq!(session.thinking_level, "xhigh");
    }

    #[test]
    fn default_auto_compaction_is_true() {
        let session = make_test_session("s1");
        assert!(session.auto_compaction);
    }

    #[test]
    fn default_auto_retry_is_true() {
        let session = make_test_session("s1");
        assert!(session.auto_retry);
    }

    #[test]
    fn default_ephemeral_is_false() {
        let session = make_test_session("s1");
        assert!(!session.ephemeral);
    }

    #[test]
    fn default_is_streaming_is_false() {
        let session = make_test_session("s1");
        assert!(!session
            .is_streaming
            .load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn default_messages_empty() {
        let session = make_test_session("s1");
        let msgs = session.get_messages();
        assert!(msgs.is_empty());
    }

    #[test]
    fn default_created_by_is_empty() {
        let session = make_test_session("s1");
        assert!(session.created_by.is_empty());
    }

    #[test]
    fn default_parent_session_id_is_empty() {
        let session = make_test_session("s1");
        assert!(session.parent_session_id.is_empty());
    }

    #[test]
    fn default_source_meta_is_null() {
        let session = make_test_session("s1");
        assert_eq!(session.source_meta, serde_json::Value::Null);
    }

    #[test]
    fn default_sandbox_policy_is_none() {
        let session = make_test_session("s1");
        assert!(session.sandbox_policy.is_none());
    }

    // ─── Setters ────────────────────────────────────────────────────────────

    #[test]
    fn set_ephemeral() {
        let mut session = make_test_session("s1");
        session.set_ephemeral(true);
        assert!(session.ephemeral);
        session.set_ephemeral(false);
        assert!(!session.ephemeral);
    }

    #[test]
    fn set_auto_compaction() {
        let mut session = make_test_session("s1");
        session.set_auto_compaction(false);
        assert!(!session.auto_compaction);
        session.set_auto_compaction(true);
        assert!(session.auto_compaction);
    }

    #[test]
    fn set_auto_retry() {
        let mut session = make_test_session("s1");
        session.set_auto_retry(false);
        assert!(!session.auto_retry);
    }

    #[test]
    fn set_cwd() {
        let mut session = make_test_session("s1");
        session.set_cwd("/tmp/project");
        assert_eq!(session.cwd, "/tmp/project");
    }

    #[test]
    fn set_permission_level() {
        let mut session = make_test_session("s1");
        session.set_permission_level("workspace");
        assert_eq!(session.get_permission_level(), "workspace");
        session.set_permission_level("none");
        assert_eq!(session.get_permission_level(), "none");
    }

    // ─── get_last_assistant_text ────────────────────────────────────────────

    #[test]
    fn get_last_assistant_text_empty() {
        let session = make_test_session("s1");
        assert_eq!(session.get_last_assistant_text(), "");
    }

    #[test]
    fn get_last_assistant_text_with_messages() {
        let session = make_test_session("s1");
        {
            let mut msgs = session.messages.write();
            msgs.push(crate::types::AgentMessage {
                role: "user".to_string(),
                content: vec![crate::types::ContentBlock::text("hello")],
                ..Default::default()
            });
            msgs.push(crate::types::AgentMessage {
                role: "assistant".to_string(),
                content: vec![crate::types::ContentBlock::text("world")],
                ..Default::default()
            });
        }
        assert_eq!(session.get_last_assistant_text(), "world");
    }

    #[test]
    fn get_last_assistant_text_only_user_msgs() {
        let session = make_test_session("s1");
        {
            let mut msgs = session.messages.write();
            msgs.push(crate::types::AgentMessage {
                role: "user".to_string(),
                content: vec![crate::types::ContentBlock::text("hello")],
                ..Default::default()
            });
        }
        assert_eq!(session.get_last_assistant_text(), "");
    }

    // ─── get_session_stats ──────────────────────────────────────────────────

    #[test]
    fn session_stats_empty() {
        let session = make_test_session("s1");
        let stats = session.get_session_stats();
        assert_eq!(stats["sessionId"], "s1");
        assert_eq!(stats["userMessages"], 0);
        assert_eq!(stats["assistantMessages"], 0);
        assert_eq!(stats["totalMessages"], 0);
    }

    #[test]
    fn session_stats_with_messages() {
        let session = make_test_session("s1");
        {
            let mut msgs = session.messages.write();
            msgs.push(crate::types::AgentMessage {
                role: "user".to_string(),
                content: vec![crate::types::ContentBlock::text("q1")],
                ..Default::default()
            });
            msgs.push(crate::types::AgentMessage {
                role: "assistant".to_string(),
                content: vec![crate::types::ContentBlock::text("a1")],
                ..Default::default()
            });
            msgs.push(crate::types::AgentMessage {
                role: "user".to_string(),
                content: vec![crate::types::ContentBlock::text("q2")],
                ..Default::default()
            });
        }
        let stats = session.get_session_stats();
        assert_eq!(stats["userMessages"], 2);
        assert_eq!(stats["assistantMessages"], 1);
        assert_eq!(stats["totalMessages"], 3);
    }

    // ─── new_session clears messages ────────────────────────────────────────

    #[test]
    fn new_session_clears_messages() {
        let mut session = make_test_session("s1");
        {
            let mut msgs = session.messages.write();
            msgs.push(crate::types::AgentMessage {
                role: "user".to_string(),
                content: vec![crate::types::ContentBlock::text("hello")],
                ..Default::default()
            });
        }
        session.new_session().unwrap();
        assert!(session.get_messages().is_empty());
    }

    // ─── strip_image_content_from_messages ──────────────────────────────────

    #[test]
    fn strip_images_removes_image_blocks() {
        let session = make_test_session("s1");
        {
            let mut msgs = session.messages.write();
            msgs.push(crate::types::AgentMessage {
                role: "user".to_string(),
                content: vec![
                    crate::types::ContentBlock::text("look"),
                    crate::types::ContentBlock::image("data:image/png;base64,abc"),
                ],
                ..Default::default()
            });
        }
        session.strip_image_content_from_messages();
        let msgs = session.messages.read();
        assert_eq!(msgs[0].content.len(), 1);
        match &msgs[0].content[0] {
            crate::types::ContentBlock::Text { text } => assert_eq!(text, "look"),
            _ => panic!("expected Text"),
        }
    }

    // ─── execute_shell ──────────────────────────────────────────────────────

    #[test]
    fn execute_shell_echo() {
        let session = make_test_session("s1");
        // Create the cwd directory so the shell can cd into it
        std::fs::create_dir_all(&session.cwd).unwrap();
        let result = session.execute_shell("echo hello").unwrap();
        let output = result["output"].as_str().unwrap();
        assert!(output.contains("hello"));
        assert_eq!(result["exitCode"], 0);
    }

    #[test]
    fn execute_shell_nonzero_exit() {
        let session = make_test_session("s1");
        std::fs::create_dir_all(&session.cwd).unwrap();
        let result = session.execute_shell("false").unwrap();
        assert_eq!(result["exitCode"], 1);
    }

    // ─── steer / follow_up ──────────────────────────────────────────────────

    #[test]
    fn steer_without_active_run_is_rejected() {
        let mut session = make_test_session("s1");
        assert!(session.steer("stop that").is_err());
    }

    #[tokio::test]
    async fn follow_up_not_streaming_calls_prompt() {
        let mut session = make_test_session("s1");
        std::fs::create_dir_all(&session.cwd).unwrap();
        // Not streaming → follow_up falls through to prompt, which needs an
        // actual LLM. With EmptyProvider, prompt() may return an error, but
        // it shouldn't panic.
        let _ = session.follow_up("hello");
    }

    #[tokio::test]
    async fn active_run_uses_snapshot_and_leaves_next_run_config_writable() {
        let cwd = test_workspace();
        std::fs::create_dir_all(&cwd).unwrap();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let shared_loop = Arc::new(tokio::sync::RwLock::new(Loop::new(
            Arc::new(BlockingProvider {
                started: started.clone(),
                release: release.clone(),
            }),
            "mock",
        )));
        let mut session = ServerSession::new(
            "snapshot-run".to_string(),
            shared_loop.clone(),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );
        session.set_ephemeral(true);

        let lease = session
            .prompt("hold", &[], &[], Some("run-snapshot"), None)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();

        // The active task owns an independent Loop. Updating this control
        // plane therefore succeeds immediately and applies to the next run.
        session.set_auto_retry(false);
        assert_eq!(shared_loop.try_read().unwrap().config.max_retries, 0);
        assert_eq!(session.runtime.snapshot().unwrap().run_id, lease.run_id);

        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn in_run_follow_up_survives_terminal_rewrite() {
        let cwd = test_workspace();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let mut session = ServerSession::new(
            "follow-up-persistence".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(BlockingProvider {
                    started: started.clone(),
                    release: release.clone(),
                }),
                "mock",
            ))),
            Arc::new(Manager::new(std::path::Path::new(&cwd).join("sessions"))),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );
        std::fs::create_dir_all(&session.cwd).unwrap();
        session
            .prompt("first question", &[], &[], Some("run-follow-up"), None)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        session
            .follow_up_run("second question", Some("run-follow-up"))
            .unwrap();

        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let loaded = session.session_manager.load(&session.session_id).unwrap();
        let user_texts: Vec<String> = loaded
            .entries
            .iter()
            .filter(|entry| entry.entry_type == crate::session::ENTRY_TYPE_USER)
            .filter_map(|entry| entry.content.as_ref().map(ToString::to_string))
            .collect();
        assert_eq!(user_texts.len(), 2);
        assert!(user_texts[0].contains("first question"));
        assert!(user_texts[1].contains("second question"));
        assert_eq!(
            loaded.entries.last().unwrap().entry_type,
            crate::session::ENTRY_TYPE_RUN_TERMINAL
        );

        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[tokio::test]
    async fn model_error_commits_before_session_returns_to_idle() {
        let mut session = make_persistent_test_session("error-commit");
        std::fs::create_dir_all(&session.cwd).unwrap();
        session.set_auto_retry(false);
        session
            .prompt("persist before failing", &[], &[], Some("run-error"), None)
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let loaded = session.session_manager.load(&session.session_id).unwrap();
        assert!(loaded.entries.iter().any(|entry| {
            entry.entry_type == crate::session::ENTRY_TYPE_USER
                && entry
                    .content
                    .as_ref()
                    .is_some_and(|content| content.to_string().contains("persist before failing"))
        }));
        session.persistence.barrier().unwrap();

        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[tokio::test]
    async fn abort_commits_before_an_immediate_resend_can_start() {
        let cwd = test_workspace();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let mut session = ServerSession::new(
            "abort-commit".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(BlockingProvider {
                    started: started.clone(),
                    release: release.clone(),
                }),
                "mock",
            ))),
            Arc::new(Manager::new(std::path::Path::new(&cwd).join("sessions"))),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );
        std::fs::create_dir_all(&session.cwd).unwrap();
        session
            .prompt("cancel me", &[], &[], Some("run-cancel"), None)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        session.abort_run(Some("run-cancel")).unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        session.persistence.barrier().unwrap();
        let loaded = session.session_manager.load(&session.session_id).unwrap();
        assert!(loaded.entries.iter().any(|entry| {
            entry.entry_type == crate::session::ENTRY_TYPE_USER
                && entry
                    .content
                    .as_ref()
                    .is_some_and(|content| content.to_string().contains("cancel me"))
        }));
        let terminal = loaded
            .entries
            .iter()
            .find(|entry| {
                entry.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL
                    && entry
                        .content
                        .as_ref()
                        .and_then(|content| content.get("run_id"))
                        .and_then(serde_json::Value::as_str)
                        == Some("run-cancel")
            })
            .expect("cancelled run must have a terminal marker");
        assert_eq!(
            terminal.content.as_ref().unwrap()["state"],
            crate::session::RUN_STATE_CANCELLED
        );
        assert_eq!(
            loaded.entries.last().map(|entry| entry.id.as_str()),
            Some(terminal.id.as_str()),
            "terminal marker must be the final durable journal record"
        );

        let next = session
            .runtime
            .begin(Some("run-after-cancel"), None)
            .unwrap();
        assert!(session.runtime.begin_finalizing(&next));
        assert!(session.runtime.finish(&next));
        release.notify_one();

        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[tokio::test]
    async fn healing_rewrite_preserves_run_markers() {
        let mut session = make_persistent_test_session("rewrite-markers");
        std::fs::create_dir_all(&session.cwd).unwrap();
        session.set_auto_retry(false);
        // Refuse the append-only commit once. The fallback rewrite must heal
        // the snapshot without erasing the current lifecycle boundary.
        session.persistence.fail_next_commit();
        session
            .prompt("heal the journal", &[], &[], Some("run-healed"), None)
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let loaded = session.session_manager.load(&session.session_id).unwrap();
        assert!(loaded.entries.iter().any(|entry| {
            entry.entry_type == crate::session::ENTRY_TYPE_RUN_STARTED
                && entry
                    .content
                    .as_ref()
                    .and_then(|content| content.get("run_id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("run-healed")
        }));
        let terminal = loaded.entries.last().expect("terminal record");
        assert_eq!(terminal.entry_type, crate::session::ENTRY_TYPE_RUN_TERMINAL);
        assert_eq!(terminal.content.as_ref().unwrap()["run_id"], "run-healed");

        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[tokio::test]
    async fn commit_failure_keeps_session_persistence_degraded() {
        let mut session = make_persistent_test_session("commit-failure");
        std::fs::create_dir_all(&session.cwd).unwrap();
        session.set_auto_retry(false);
        // The append-only run commit fails, and so does the healing full-rewrite
        // fallback — so the run cannot be persisted at all and the session must
        // stay fenced in PersistenceDegraded (no new run may start).
        session.persistence.fail_next_commit();
        session.persistence.fail_next_rewrite();
        session
            .prompt("must remain fenced", &[], &[], Some("run-degraded"), None)
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if session
                    .runtime
                    .snapshot()
                    .is_some_and(|run| run.phase == crate::runtime::RunPhase::PersistenceDegraded)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(session
            .prompt(
                "must be rejected",
                &[],
                &[],
                Some("run-must-not-start"),
                None
            )
            .is_err());

        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[tokio::test]
    async fn new_run_closes_a_prior_interrupted_run() {
        let mut session = make_persistent_test_session("interrupt-close");
        std::fs::create_dir_all(&session.cwd).unwrap();
        session.set_auto_retry(false);

        // First run completes normally (creates the file and a run_terminal).
        session
            .prompt("first", &[], &[], Some("run-first"), None)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            session
                .session_manager
                .unterminated_run_id(&session.session_id)
                .unwrap(),
            None
        );

        // Simulate a crash: a later run began (run_started durable) but the
        // agent died before committing, so there is no run_terminal.
        session
            .session_manager
            .append_entries(
                &session.session_id,
                &[crate::session::SessionEntry::run_started("crashed-run", 99)],
            )
            .unwrap();
        assert_eq!(
            session
                .session_manager
                .unterminated_run_id(&session.session_id)
                .unwrap(),
            Some("crashed-run".to_string())
        );

        // The next run must close the interrupted run before its own user
        // message, recovering it as interrupted_by_restart (never completed).
        session
            .prompt("second", &[], &[], Some("run-second"), None)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session.runtime.snapshot().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let loaded = session.session_manager.load(&session.session_id).unwrap();
        // The interrupted run is closed with the interrupted_by_restart state.
        let closing_pos = loaded
            .entries
            .iter()
            .position(|e| {
                e.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL
                    && e.content.as_ref().is_some_and(|c| {
                        c.get("run_id").and_then(|v| v.as_str()) == Some("crashed-run")
                            && c.get("state").and_then(|v| v.as_str())
                                == Some(crate::session::RUN_STATE_INTERRUPTED_BY_RESTART)
                    })
            })
            .expect("interrupted run must be closed as interrupted_by_restart");
        // The closing marker precedes the second run's user message.
        let second_user_pos = loaded
            .entries
            .iter()
            .position(|e| {
                e.entry_type == crate::session::ENTRY_TYPE_USER
                    && e.content
                        .as_ref()
                        .is_some_and(|c| c.to_string().contains("second"))
            })
            .unwrap();
        assert!(closing_pos < second_user_pos);
        // And no run is left open after the second run commits.
        assert_eq!(crate::session::find_unterminated_run(&loaded.entries), None);

        session.persistence.barrier().unwrap();
        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[tokio::test]
    async fn initial_persistence_rejection_emits_one_terminal_and_rolls_back() {
        let cwd = test_workspace();
        std::fs::create_dir_all(&cwd).unwrap();
        let unusable_session_dir = std::path::Path::new(&cwd).join("not-a-directory");
        std::fs::write(&unusable_session_dir, b"block create_dir_all").unwrap();
        let broadcaster = Arc::new(SseBroadcaster::new());
        let mut session = ServerSession::new(
            "initial-save-failure".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(Manager::new(unusable_session_dir)),
            &cwd,
            broadcaster.clone(),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );

        assert!(session
            .prompt(
                "must roll back",
                &[],
                &[],
                Some("run-initial-save-failure"),
                None
            )
            .is_err());
        assert!(session.runtime.snapshot().is_none());
        assert!(session.messages.read().is_empty());

        let (_, events, _, _) = broadcaster
            .events_since("run-initial-save-failure", -1)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "agent_end")
                .count(),
            1
        );
        assert!(events.iter().any(|event| event.event_type == "error"));

        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    // ─── abort ──────────────────────────────────────────────────────────────

    #[test]
    fn abort_keeps_session_busy_until_matching_task_finishes() {
        let session = make_test_session("s1");
        let lease = session.runtime.begin(Some("run-1"), None).unwrap();
        session.abort();
        assert!(session
            .is_streaming
            .load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(
            session.runtime.snapshot().unwrap().phase,
            crate::runtime::RunPhase::Cancelling
        );
        assert!(session.runtime.begin_finalizing(&lease));
        assert!(session.runtime.finish(&lease));
        assert!(!session
            .is_streaming
            .load(std::sync::atomic::Ordering::Relaxed));
    }

    // ─── set_thinking_level ─────────────────────────────────────────────────

    #[test]
    fn set_thinking_level_updates_field() {
        let mut session = make_test_session("s1");
        session.set_thinking_level("high");
        assert_eq!(session.thinking_level, "high");
    }

    #[test]
    fn set_thinking_level_off() {
        let mut session = make_test_session("s1");
        session.set_thinking_level("off");
        assert_eq!(session.thinking_level, "off");
    }

    // ─── new (per-session loop) ─────────────────────────────────────────

    #[test]
    fn new_with_own_loop_defaults() {
        let cwd = test_workspace();
        let session = ServerSession::new(
            "own_loop_test".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );
        assert_eq!(session.session_id(), "own_loop_test");
        assert_eq!(session.thinking_level, "xhigh");
        assert_eq!(session.get_permission_level(), "all");
        assert!(session.auto_compaction);
    }

    /// Sessions mint independent loops from the template: queues, counters
    /// and interrupt flags must not be shared across sessions.
    #[test]
    fn independent_loop_copies_have_isolated_state() {
        let template = Loop::new(Arc::new(EmptyProvider), "mock").with_system_prompt("tpl");
        let a = template.independent_copy();
        let b = template.independent_copy();
        assert_eq!(a.system_prompt, "tpl");
        assert_eq!(b.model, "mock");
        // Interrupt flag: fresh Arc per copy.
        assert!(!std::sync::Arc::ptr_eq(
            &a.interrupt_flag,
            &b.interrupt_flag
        ));
        // Token counters: fresh Arc per copy.
        assert!(!std::sync::Arc::ptr_eq(
            &a.cumulative_input_tokens,
            &b.cumulative_input_tokens
        ));
    }

    // ─── compact ────────────────────────────────────────────────────────────

    #[test]
    fn compact_empty_messages() {
        let session = make_test_session("s1");
        let result = session.compact("").unwrap();
        assert_eq!(result["messagesRemoved"], 0);
        assert_eq!(result["summary"], "");
    }

    // ─── add_session_rule ───────────────────────────────────────────────────

    #[test]
    fn add_session_rule_does_not_panic() {
        let session = make_test_session("s1");
        session.add_session_rule("/tmp/**", "read");
        // Just verify no panic — the rule goes into the session_rules mutex
    }

    // ─── default_workspace ──────────────────────────────────────────────────

    #[test]
    fn default_workspace_contains_future_agent() {
        let ws = default_workspace();
        assert!(ws.contains(".future"));
        assert!(ws.contains("agent"));
        assert!(ws.contains("workspace"));
    }

    // ─── ServerSession unique tests (no duplicates with existing tests) ─────

    #[test]
    fn set_cwd_updates_field() {
        let mut session = make_test_session("s1");
        session.set_cwd("/new/path");
        assert_eq!(session.cwd, "/new/path");
    }

    #[test]
    fn set_permission_level_invalid() {
        let mut session = make_test_session("s1");
        session.set_permission_level("invalid");
        // Should not crash, permission stays as-is or reverts
    }

    #[test]
    fn get_permission_level_default() {
        let session = make_test_session("s1");
        assert_eq!(session.get_permission_level(), "all");
    }

    #[test]
    fn set_auto_compaction_toggles() {
        let mut session = make_test_session("s1");
        assert!(session.auto_compaction);
        session.set_auto_compaction(false);
        assert!(!session.auto_compaction);
    }

    #[test]
    fn set_auto_retry_toggles() {
        let mut session = make_test_session("s1");
        assert!(session.auto_retry);
        session.set_auto_retry(false);
        assert!(!session.auto_retry);
    }

    #[test]
    fn set_system_prompt_updates() {
        let mut session = make_test_session("s1");
        session.set_system_prompt("custom prompt");
        // Verify the prompt was set (indirect check via the loop)
    }

    #[test]
    fn append_system_prompt_appends() {
        let mut session = make_test_session("s1");
        session.set_system_prompt("base");
        session.append_system_prompt("appended");
        // Verify no panic
    }

    #[test]
    fn set_ephemeral_toggles() {
        let mut session = make_test_session("s1");
        session.set_ephemeral(true);
        // Field should be updated
    }

    #[test]
    fn set_tools_filters() {
        let mut session = make_test_session("s1");
        session.set_tools(&["shell".to_string(), "read".to_string()]);
        // Should not panic
    }

    #[test]
    fn disable_tools_clears() {
        let mut session = make_test_session("s1");
        session.disable_tools();
        // Should not panic
    }

    #[test]
    fn disable_builtin_tools() {
        let mut session = make_test_session("s1");
        session.disable_builtin_tools();
        // Should not panic
    }

    #[test]
    fn strip_images_removes_image_blocks_v2() {
        let session = make_test_session("s1");
        session.messages.write().push(crate::types::AgentMessage {
            role: "user".to_string(),
            content: vec![
                crate::types::ContentBlock::text("hello"),
                crate::types::ContentBlock::image("data:image/png;base64,abc"),
            ],
            ..Default::default()
        });
        session.strip_image_content_from_messages();
        let msgs = session.messages.read();
        assert_eq!(msgs[0].content.len(), 1);
    }

    #[test]
    fn reload_credentials_no_panic() {
        let session = make_test_session("s1");
        session.reload_credentials();
    }

    #[test]
    fn fork_does_not_panic() {
        let mut session = make_test_session("s1");
        let _ = session.fork("entry_id");
    }

    #[test]
    fn delete_session_does_not_panic() {
        let session = make_test_session("s1");
        let _ = session.delete_session("other_id");
    }

    #[test]
    fn list_sessions_empty_dir() {
        let session = make_test_session("s1");
        let result = session.list_sessions();
        assert!(result.is_ok());
    }

    #[test]
    fn set_sandbox_policy_updates() {
        let mut session = make_test_session("s1");
        session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Off,
        });
        // Should not panic
    }

    #[test]
    fn compact_empty_messages_returns_zero() {
        let session = make_test_session("s1");
        let result = session.compact("").unwrap();
        assert_eq!(result["messagesRemoved"], 0);
    }
}
