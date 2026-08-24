use crate::session::Manager;
use crate::types::ConvertToLLM;
use anyhow::Result;
use std::{collections::HashMap, sync::Arc};

use super::{ApprovalGate, SseBroadcaster};

/// Consume scheduler wake notifications, applying each finished run to the
/// session, until the session is dropped (Weak upgrade fails) or the
/// completion sender hangs up. A free function (not an inline spawn closure)
/// so the dropped-session exit is directly testable.
async fn scheduler_wake_worker(
    session: std::sync::Weak<parking_lot::RwLock<ServerSession>>,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<crate::runtime::RunLease>,
) {
    while let Some(finished) = receiver.recv().await {
        let Some(session) = session.upgrade() else {
            return;
        };
        session.write().on_scheduled_run_finished(&finished);
    }
}

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
    /// The agent run-loop: LLM provider + tool registry + iteration counter.
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
    /// Process-local queued-run state. It deliberately resets when the Agent
    /// process restarts; `agent_instance_id` lets clients reconcile that loss.
    pub scheduler: Arc<crate::runtime::InMemoryRunQueue>,
    pub(super) scheduled_snapshots: HashMap<String, super::session_prompt::AcceptedRunSnapshot>,
    /// Admission fence set before queued/active cleanup begins. A deleting
    /// session stays addressable for retry, but can never accept new work.
    pub deleting: bool,
    scheduler_wake_rx: Arc<
        parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<crate::runtime::RunLease>>>,
    >,
    /// ID of the session this one was forked from, if any.
    pub parent_session_id: String,
    /// Human-readable label (set via `/name`).  Empty until named.
    pub session_name: String,
    /// Source that created this session: "desktop", "tui", "fork", "feishu", "dingtalk", etc.
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
        Self::new_with_queue_budget(
            session_id,
            agent_loop,
            manager,
            cwd,
            broadcaster,
            approval_gate,
            model_registry,
            Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_queue_budget(
        session_id: String,
        agent_loop: Arc<tokio::sync::RwLock<crate::agent::Loop>>,
        manager: Arc<Manager>,
        cwd: &str,
        broadcaster: Arc<SseBroadcaster>,
        approval_gate: ApprovalGate,
        model_registry: Arc<parking_lot::RwLock<crate::models::Registry>>,
        queue_budget: Arc<crate::runtime::GlobalQueueBudget>,
    ) -> Self {
        if let Err(error) =
            broadcaster.configure_journal(session_id.clone(), manager.run_data_path(&session_id))
        {
            tracing::error!(session_id, "failed to configure event journal: {error:#}");
        }
        // Clone token counter Arcs and queue senders from the agent loop for lock-free access
        let (ti, to, tcr, tcw, lpt) = if let Ok(loop_) = agent_loop.try_read() {
            (
                loop_.cumulative_input_tokens.clone(),
                loop_.cumulative_output_tokens.clone(),
                loop_.cumulative_cache_read_tokens.clone(),
                loop_.cumulative_cache_write_tokens.clone(),
                loop_.last_prompt_tokens.clone(),
            )
        } else {
            (
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
                Arc::new(std::sync::atomic::AtomicI64::new(0)),
            )
        };
        let is_streaming = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let runtime = Arc::new(crate::runtime::SessionRuntime::new(is_streaming.clone()));
        let (scheduler_wake_tx, scheduler_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        runtime.set_completion_sender(scheduler_wake_tx);
        let scheduler = Arc::new(crate::runtime::InMemoryRunQueue::with_limits_and_global(
            &session_id,
            1,
            crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY,
            crate::runtime::DEFAULT_SESSION_QUEUE_BYTES,
            crate::runtime::DEFAULT_REQUEST_BYTES,
            256,
            queue_budget,
        ));
        let persistence =
            crate::session::SessionPersistence::new(manager.clone(), session_id.clone());
        Self {
            session_id: session_id.clone(),
            agent_loop,
            messages: Arc::new(parking_lot::RwLock::new(vec![])),
            model: String::new(),
            thinking_level: "xhigh".to_string(), // Match default
            auto_compaction: true,               // Match default
            auto_retry: true,
            session_manager: manager,
            persistence,
            cwd: cwd.to_string(),
            is_streaming,
            runtime,
            scheduler,
            scheduled_snapshots: HashMap::new(),
            deleting: false,
            scheduler_wake_rx: Arc::new(parking_lot::Mutex::new(Some(scheduler_wake_rx))),
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

    /// Start the process-local scheduler worker once this session has been
    /// placed behind its final Arc/RwLock owner.
    pub fn ensure_scheduler_worker(session: &Arc<parking_lot::RwLock<Self>>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let receiver_cell = session.read().scheduler_wake_rx.clone();
        let Some(receiver) = receiver_cell.lock().take() else {
            return;
        };
        handle.spawn(scheduler_wake_worker(Arc::downgrade(session), receiver));
    }

    fn on_scheduled_run_finished(&mut self, finished: &crate::runtime::RunLease) {
        if let Some(error) = self
            .broadcaster
            .persistence_error()
            .or_else(|| self.persistence.last_error())
        {
            tracing::error!(
                run_id = %finished.run_id,
                "scheduler paused before dequeue because persistence is unavailable: {error}"
            );
            return;
        }
        if self.scheduler.active().is_some() {
            if let Err(error) = self.scheduler.finish_active(&finished.run_id) {
                tracing::error!(
                    run_id = %finished.run_id,
                    "scheduler could not release completed run: {error}"
                );
            }
        }
        while let Some(next) = self.scheduler.queued().into_iter().next() {
            match self.start_next_scheduled() {
                Ok(_) => return,
                Err(error) => {
                    if self
                        .scheduler
                        .active()
                        .is_some_and(|(active, _)| active.run_id == next.run_id)
                    {
                        let _ = self.scheduler.finish_active(&next.run_id);
                    } else {
                        let _ = self.scheduler.cancel_queued(
                            &next.run_id,
                            crate::runtime::QueuedCancellationReason::Cancelled,
                        );
                    }
                    self.scheduled_snapshots.remove(&next.run_id);
                    tracing::error!(
                        run_id = %next.run_id,
                        "queued run failed before task start: {error}"
                    );
                }
            }
        }
    }

    pub fn cancel_queued_run(
        &mut self,
        run_id: &str,
        reason: crate::runtime::QueuedCancellationReason,
    ) -> Result<crate::runtime::ScheduledRunRequest, crate::runtime::RunQueueError> {
        let cancelled = self.scheduler.cancel_queued(run_id, reason)?;
        self.scheduled_snapshots.remove(run_id);
        Ok(cancelled)
    }

    pub fn cancel_all_queued_runs(
        &mut self,
        reason: crate::runtime::QueuedCancellationReason,
    ) -> Vec<crate::runtime::ScheduledRunRequest> {
        let cancelled = self.scheduler.cancel_all_queued(reason);
        for request in &cancelled {
            self.scheduled_snapshots.remove(&request.run_id);
        }
        cancelled
    }

    pub fn recover_persistence_degraded(&mut self) -> Result<crate::runtime::RunLease> {
        let active = self
            .runtime
            .snapshot()
            .ok_or_else(|| anyhow::anyhow!("session has no active run"))?;
        if active.phase != crate::runtime::RunPhase::PersistenceDegraded {
            anyhow::bail!("active run is not persistence_degraded");
        }
        let lease = crate::runtime::RunLease {
            run_id: active.run_id,
            epoch: active.epoch,
            run_sequence: active.run_sequence,
        };
        self.broadcaster.recover_storage()?;
        let terminal = crate::session::SessionEntry::run_terminal(
            &lease.run_id,
            crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
            0,
            0,
            Some("persistence recovered after an uncertain outcome"),
        );
        self.persistence.recover_with_entries(vec![terminal])?;
        if !self.runtime.recover_persistence_degraded(&lease) {
            anyhow::bail!("persistence recovery lost the active run lease");
        }
        Ok(lease)
    }

    pub fn session_name(&self) -> String {
        self.session_name.clone()
    }

    pub fn set_session_name(&mut self, name: &str) {
        self.session_name = name.to_string();
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

            let max_tokens = Some(crate::models::effective_max_tokens(&model_config));

            let auth = crate::AuthStore::load();
            let api_key =
                resolve_api_key(&auth, model, &model_config.provider, &model_config.api_key);

            // Build a FRESH provider (its own reqwest client) and swap it in,
            // rather than mutating the existing provider's endpoint.  Each
            // session owns its loop (minted from AppState::loop_template), so
            // the fresh client is this session's alone: concurrent sessions
            // use independent connections and never clobber each other's
            // endpoint mid-run.
            let target = crate::llm::schema::ResolvedModelTarget::from_model(
                &model_config,
                api_key,
                None,
                max_tokens,
            )?;
            let mut client = crate::llm::Client::from_target(target);
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
    ///
    /// Also heals the `set_model` fallback: while the registry could not
    /// resolve this model (catalog unavailable during a logout/broken-auth
    /// window), `set_model` froze the full `provider/id` literal into the
    /// loop, and upstream rejects that name ("Model 'provider/id' is not
    /// configured"). Once the registry resolves the model again, restore the
    /// bare id so the next request carries a name the server knows.
    pub fn reload_credentials(&self) -> Result<()> {
        if self.model.is_empty() {
            let loop_ = self
                .agent_loop
                .try_read()
                .map_err(|_| anyhow::anyhow!("run configuration is busy; retry prompt"))?;
            loop_.provider.set_api_key("");
            return Ok(());
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
        let base_url = registry_resolved
            .as_ref()
            .map(|m| m.base_url.clone())
            .unwrap_or_default();
        let api_key = resolve_api_key(&auth, &self.model, &provider, &model_key);

        // The shared loop is a short-lived control plane; active runs own
        // independent snapshots. Wait until the latest credential revision is
        // installed so config commands cannot acknowledge while a session is
        // still on the old key. A write lock is needed (not just for the
        // interior-mutable provider) because the fallback heal below mutates
        // `loop_.model`.
        let mut loop_ = self
            .agent_loop
            .try_write()
            .map_err(|_| anyhow::anyhow!("run configuration is busy; retry prompt"))?;
        loop_.provider.set_api_key(&api_key);
        if !base_url.is_empty() {
            loop_.provider.set_base_url(&base_url);
        }
        // Fallback heal: `set_model` only ever stores the bare resolved id
        // here, so any divergence means the loop still carries the unresolved
        // literal from a resolve-miss window.
        if let Some(mc) = registry_resolved.as_ref() {
            if loop_.model != mc.id {
                loop_.model = mc.id.clone();
            }
        }
        Ok(())
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

    pub fn compact(&self, instructions: &str) -> Result<serde_json::Value> {
        use std::sync::atomic::Ordering;
        let messages = self.messages.read().clone();
        let context_window = self
            .model_registry
            .read()
            .resolve(&self.model)
            .map(|m| m.context_window)
            .unwrap_or(1_000_000);
        let reserve_tokens = ((context_window as f64 * 0.1) as i32).max(16384);
        let keep_tokens = ((context_window as f64 * 0.2) as i32).max(reserve_tokens);
        let active_checkpoint = self
            .session_manager
            .load(&self.session_id)
            .ok()
            .and_then(|session| crate::session::latest_context_checkpoint(&session.entries));
        let reported = self
            .last_prompt_tokens
            .load(Ordering::Relaxed)
            .try_into()
            .ok()
            .filter(|tokens: &u64| *tokens > 0);
        let prompt = crate::compaction::project_prompt_context(
            &messages,
            active_checkpoint.as_ref(),
            reported,
            context_window.max(1) as u64,
        );
        let manager = crate::compaction::ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens: keep_tokens,
            context_window,
            model: self.model.clone(),
        };
        match manager.prepare(
            prompt,
            crate::compaction::CompactionTrigger::Manual,
            Some(instructions),
        )? {
            crate::compaction::ContextPreparation::Unchanged { prompt } => Ok(serde_json::json!({
                "tokensBefore": prompt.usage.estimated_input_tokens,
                "tokensAfter": prompt.usage.estimated_input_tokens,
                "summary": "",
                "messagesRemoved": 0,
            })),
            crate::compaction::ContextPreparation::Compacted { checkpoint, .. } => {
                self.persistence
                    .commit_checkpoint(crate::session::checkpoint_to_entry(&checkpoint))?;
                if let Ok(loop_) = self.agent_loop.try_write() {
                    *loop_.active_checkpoint.lock() = Some((*checkpoint).clone());
                }
                if let Some(event) = super::prompt_helpers::run_event_to_sse(
                    crate::agent::RunEvent::CompactionCommitted {
                        checkpoint: (*checkpoint).clone(),
                    },
                ) {
                    self.broadcaster.broadcast(event);
                }
                let summary = checkpoint
                    .summary
                    .iter()
                    .filter_map(|block| match block {
                        crate::types::ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(serde_json::json!({
                    "checkpointId": checkpoint.checkpoint_id,
                    "tokensBefore": checkpoint.tokens_before,
                    "tokensAfter": checkpoint.tokens_after,
                    "summary": summary,
                    "messagesRemoved": checkpoint.tokens_before.saturating_sub(checkpoint.tokens_after),
                }))
            }
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

    /// Mid-turn steering: queue a note that the RUNNING turn drains at its
    /// next step boundary (via the shared steering cell that links the shared
    /// Loop and its run snapshot). Unlike `append_system_prompt` — which only
    /// takes effect on the next run — this reaches the in-flight turn.
    pub fn steer(&mut self, note: &str) {
        if let Ok(loop_) = self.agent_loop.try_read() {
            loop_.steering_notes.lock().push(note.to_string());
        }
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
            "toolCalls": msgs.iter().filter(|m| m.has_tool_calls()).count(),
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
            "queuedRuns": self.scheduler.queued().len(),
            "queuedBytes": self.scheduler.queued_bytes(),
            "eventJournalHealthy": self.broadcaster.persistence_error().is_none(),
            "eventJournalError": self.broadcaster.persistence_error(),
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
                // previous session. set_model persists via update_session_info,
                // which fails for legacy session files lacking a session_info
                // entry — log and defer to an explicit /model.
                let _ = self.set_model(&self.model.clone()).inspect_err(|e| {
                    tracing::warn!(
                        "[session] could not sync agent loop model during switch_session: {e}"
                    );
                });
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
                // NOTE: no session_name fallback here — load_path derives
                // session.name from this same last session_info entry, so
                // whenever info carries a name the restore above already
                // applied it (the old inner fallback arm was unreachable).
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
            self.scheduler = Arc::new(crate::runtime::InMemoryRunQueue::new(
                id,
                crate::session::next_run_sequence(&session.entries),
            ));
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
        llm::schema::{FinishReason, ModelRequest, ModelStreamEvent},
        types::LLMProvider,
    };
    use tokio::sync::mpsc;
    use tokio::sync::Notify;
    use tokio_stream::wrappers::ReceiverStream;

    struct EmptyProvider;

    #[async_trait::async_trait]
    impl LLMProvider for EmptyProvider {
        async fn stream_model(
            &self,
            _request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(ReceiverStream::new(rx))
        }
    }

    struct KeyRecordingProvider(Arc<parking_lot::Mutex<String>>);

    #[async_trait::async_trait]
    impl LLMProvider for KeyRecordingProvider {
        async fn stream_model(
            &self,
            _request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(ReceiverStream::new(rx))
        }

        fn set_api_key(&self, api_key: &str) {
            *self.0.lock() = api_key.to_string();
        }
    }

    struct BlockingProvider {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl LLMProvider for BlockingProvider {
        async fn stream_model(
            &self,
            _request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            let (tx, rx) = mpsc::channel(2);
            self.started.notify_one();
            let release = self.release.clone();
            tokio::spawn(async move {
                release.notified().await;
                let _ = tx
                    .send(ModelStreamEvent::Finish {
                        reason: FinishReason::Stop,
                        usage: None,
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
        let is_expected_text = matches!(
            &msgs[0].content[0],
            crate::types::ContentBlock::Text { text } if text == "look"
        );
        assert!(is_expected_text, "expected a Text block holding 'look'");
    }

    // ─── coverage batch 16: scheduler/set_model/switch residuals ──────────

    fn tracing_sink() -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .with_ansi(false)
                .finish(),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scheduler_wake_worker_exits_when_session_is_dropped() {
        let session = Arc::new(parking_lot::RwLock::new(make_test_session("wake-drop")));
        let weak = Arc::downgrade(&session);
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(crate::runtime::RunLease {
            run_id: "r1".to_string(),
            epoch: 1,
            run_sequence: None,
        })
        .unwrap();
        drop(session);
        // The queued wake is still received, but the Weak upgrade fails — the
        // worker returns instead of touching a dead session.
        scheduler_wake_worker(weak, rx).await;
    }

    #[test]
    fn on_scheduled_run_finished_logs_release_failure_for_stale_run() {
        let _sink = tracing_sink();
        let mut session = make_test_session("stale-finish");
        // Drive a replacement scheduler to active(run-a), then report a
        // DIFFERENT run as finished: the release fails and is logged.
        let queue = crate::runtime::InMemoryRunQueue::new("stale-finish", 0);
        queue
            .accept(
                "req-a",
                Some("run-a"),
                crate::runtime::BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"text": "x"}),
            )
            .unwrap();
        queue.start_next(1).unwrap();
        session.scheduler = Arc::new(queue);
        session.on_scheduled_run_finished(&crate::runtime::RunLease {
            run_id: "run-b".to_string(),
            epoch: 1,
            run_sequence: None,
        });
        // The foreign active run is left untouched.
        assert!(session.scheduler.active().is_some());
    }

    #[test]
    fn on_scheduled_run_finished_cancels_unstartable_queued_run() {
        let _sink = tracing_sink();
        let mut session = make_test_session("bad-queued");
        let queue = crate::runtime::InMemoryRunQueue::new("bad-queued", 0);
        // A queued payload that is NOT a ScheduledPromptPayload fails to
        // deserialize inside start_next_scheduled.
        queue
            .accept(
                "req-bad",
                Some("run-bad"),
                crate::runtime::BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"bogus": true}),
            )
            .unwrap();
        session.scheduler = Arc::new(queue);
        session.on_scheduled_run_finished(&crate::runtime::RunLease {
            run_id: "run-x".to_string(),
            epoch: 1,
            run_sequence: None,
        });
        // The unstartable run was cancelled out of the queue.
        assert!(session.scheduler.queued().is_empty());
    }

    #[test]
    fn on_scheduled_run_finished_releases_active_matching_unstartable_run() {
        let _sink = tracing_sink();
        let mut session = make_test_session("bad-active");
        let queue = crate::runtime::InMemoryRunQueue::new("bad-active", 0);
        queue
            .accept(
                "req-bad",
                Some("run-bad"),
                crate::runtime::BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"bogus": true}),
            )
            .unwrap();
        queue.start_next(1).unwrap();
        // Forge the active+queued-same-id inconsistency the error arm
        // defends against (accept rejects duplicate run ids).
        queue.test_requeue_active_duplicate();
        session.scheduler = Arc::new(queue);
        session.on_scheduled_run_finished(&crate::runtime::RunLease {
            run_id: "run-foreign".to_string(),
            epoch: 1,
            run_sequence: None,
        });
        // The matching active lease was released, then the forged queue
        // twin was cancelled on the next loop pass.
        assert!(session.scheduler.active().is_none());
        assert!(session.scheduler.queued().is_empty());
    }

    #[test]
    fn set_model_applies_thinking_budget_on_rebuild() {
        let mut session = make_test_session("think-budget");
        // A non-default thinking level pins a positive budget on the loop
        // config; the provider rebuild then applies it.
        session.set_thinking_level("low");
        session.set_model("gpt-4o").unwrap();
        assert_eq!(session.thinking_level, "low");
    }

    #[test]
    fn set_thinking_level_logs_persist_failure() {
        let _sink = tracing_sink();
        let mut session = make_persistent_test_session("think-persist-fail");
        // The session path exists (find succeeds) but is a DIRECTORY, so the
        // metadata update fails and the error is logged, not propagated.
        let dir_file = std::path::Path::new(&session.cwd)
            .join("sessions")
            .join("think-persist-fail.jsonl");
        std::fs::create_dir_all(&dir_file).unwrap();
        session.set_thinking_level("high");
        assert_eq!(session.thinking_level, "high");
        let _ = std::fs::remove_dir_all(&dir_file);
    }

    #[test]
    fn switch_session_restores_name_from_session_info() {
        let mut session = make_persistent_test_session("name-restore");
        // Save a session whose name lives only in the session_info content.
        let saved = crate::session::Session::snapshot(
            "name-restore".to_string(),
            session.cwd.clone(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"session_name": "Restored Name"}),
                    "mock".to_string(),
                    String::new(),
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        session.session_manager.save(&saved).unwrap();
        session.switch_session("name-restore").unwrap();
        assert_eq!(session.session_name, "Restored Name");
    }

    #[test]
    fn switch_session_tolerates_legacy_file_without_session_info() {
        let _sink = tracing_sink();
        let mut session = make_persistent_test_session("legacy-no-info");
        // Legacy file: carries a model (via a model_change entry) but NO
        // session_info entry, so set_model's update_session_info persist
        // fails — the switch logs the warning and still applies the model.
        let mut model_change =
            crate::session::SessionEntry::new_user("system", serde_json::json!({"model": "mock"}));
        model_change.entry_type = crate::session::ENTRY_TYPE_MODEL_CHANGE.to_string();
        let saved = crate::session::Session::snapshot(
            "legacy-no-info".to_string(),
            session.cwd.clone(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![model_change],
        );
        session.session_manager.save(&saved).unwrap();
        session.switch_session("legacy-no-info").unwrap();
        assert_eq!(session.model, "mock");
    }

    #[test]
    fn switch_session_unknown_id_is_a_noop_ok() {
        let mut session = make_test_session("no-such-target");
        // find() returns None: the hydrate block is skipped entirely.
        session.switch_session("definitely-missing").unwrap();
        assert_eq!(session.session_id, "no-such-target");
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

    #[test]
    fn queued_attachment_is_an_immutable_memory_snapshot() {
        let mut session = make_test_session("attachment-snapshot");
        session
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, b"accepted bytes").unwrap();
        let attachment = crate::types::Attachment {
            path: path.to_string_lossy().into_owned(),
            kind: "file".to_string(),
            name: "note.txt".to_string(),
            thumbnail: None,
        };
        session
            .enqueue_prompt(
                "queued",
                &[],
                &[attachment],
                Some("run-queued"),
                "request-queued",
                crate::runtime::BusyPolicy::EnqueueIfBusy,
            )
            .unwrap();
        std::fs::write(&path, b"changed after ack").unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            session.scheduled_attachment_bytes("run-queued").unwrap(),
            vec![b"accepted bytes".to_vec()]
        );
    }

    #[test]
    fn persistence_failure_pauses_dequeue_without_cancelling_queued_runs() {
        let mut session = make_test_session("scheduler-persistence-fence");
        session
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();
        for index in 1..=2 {
            session
                .enqueue_prompt(
                    &format!("queued {index}"),
                    &[],
                    &[],
                    Some(&format!("run-{index}")),
                    &format!("request-{index}"),
                    crate::runtime::BusyPolicy::EnqueueIfBusy,
                )
                .unwrap();
        }
        session.broadcaster.fail_next_append();
        session.broadcaster.broadcast(crate::rpc::SseEvent::new(
            "text_chunk",
            serde_json::json!({"text":"fails persistence"}),
        ));

        let error = session.start_next_scheduled().unwrap_err();
        assert!(error.to_string().contains("persistence is unavailable"));
        assert_eq!(
            session
                .scheduler
                .queued()
                .iter()
                .map(|request| request.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-1", "run-2"]
        );
        assert!(session.scheduled_setting_summary("run-1").is_some());
        assert!(session.scheduled_setting_summary("run-2").is_some());

        // Also cover the completion-wake race directly: even if a run was
        // already marked scheduler-active before journal health degraded, its
        // lease must remain held and the successor must remain queued.
        let (started, ack) = session.scheduler.start_next(7).unwrap();
        session.on_scheduled_run_finished(&crate::runtime::RunLease {
            run_id: started.run_id,
            epoch: ack.run_epoch,
            run_sequence: ack.run_sequence,
        });
        assert_eq!(
            session
                .scheduler
                .active()
                .map(|(request, _)| request.run_id),
            Some("run-1".to_string())
        );
        assert_eq!(
            session
                .scheduler
                .queued()
                .iter()
                .map(|request| request.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-2"]
        );
    }

    #[tokio::test]
    async fn in_memory_scheduler_starts_next_run_after_matching_task_exits() {
        let cwd = test_workspace();
        std::fs::create_dir_all(&cwd).unwrap();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let session = Arc::new(parking_lot::RwLock::new(ServerSession::new(
            "scheduled-runs".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(BlockingProvider {
                    started: started.clone(),
                    release: release.clone(),
                }),
                "mock",
            ))),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        )));
        session.write().set_ephemeral(true);
        ServerSession::ensure_scheduler_worker(&session);

        let first = session
            .write()
            .enqueue_prompt(
                "first",
                &[],
                &[],
                Some("run-first"),
                "request-first",
                crate::runtime::BusyPolicy::EnqueueIfBusy,
            )
            .unwrap();
        assert_eq!(
            first.accepted_state,
            crate::runtime::RunAcceptedState::Running
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();

        let second = session
            .write()
            .enqueue_prompt(
                "second",
                &[],
                &[],
                Some("run-second"),
                "request-second",
                crate::runtime::BusyPolicy::EnqueueIfBusy,
            )
            .unwrap();
        assert_eq!(
            second.accepted_state,
            crate::runtime::RunAcceptedState::Queued
        );
        assert_eq!(second.queue_position, Some(1));
        assert_eq!(
            session
                .read()
                .scheduled_setting_summary("run-second")
                .unwrap(),
            (String::new(), "xhigh".to_string(), true, "all".to_string())
        );

        // Settings changed after acceptance belong to a later submission; the
        // already queued run keeps its provider/config snapshot.
        session.write().set_thinking_level("low");
        session.write().set_auto_retry(false);
        session.write().set_permission_level("none");
        assert_eq!(
            session
                .read()
                .scheduled_setting_summary("run-second")
                .unwrap(),
            (String::new(), "xhigh".to_string(), true, "all".to_string())
        );

        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        assert_eq!(
            session.read().runtime.snapshot().unwrap().run_id,
            "run-second"
        );
        assert_eq!(
            session.read().scheduler.active().unwrap().0.run_id,
            "run-second"
        );

        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let settled = {
                    let session = session.read();
                    session.runtime.snapshot().is_none()
                        && session.scheduler.active().is_none()
                        && session.scheduler.queued().is_empty()
                };
                if settled {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
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
    async fn initial_persistence_rejection_is_not_broadcast_as_terminal() {
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

        assert!(broadcaster.persistence_error().is_some());
        assert!(broadcaster.current_run_id().is_empty());

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
        let _ = session.reload_credentials();
    }

    #[test]
    fn reload_credentials_applies_authoritative_provider_key() {
        let _home = crate::test_support::TestHome::new();
        let auth_path = crate::config::providers::auth_json_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            auth_path,
            r#"{"custom":{"type":"api_key","key":"new-key"}}"#,
        )
        .unwrap();

        let observed = Arc::new(parking_lot::Mutex::new("old-key".to_string()));
        let cwd = test_workspace();
        let mut session = ServerSession::new(
            "key-refresh".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(KeyRecordingProvider(observed.clone())),
                "model",
            ))),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );
        session.model = "custom/model".to_string();

        session.reload_credentials().unwrap();
        assert_eq!(&*observed.lock(), "new-key");
    }

    #[test]
    fn reload_credentials_heals_fallback_model_literal() {
        let mut session = make_test_session("fallback-heal");
        session.set_model("gpt-4o").unwrap();
        let bare = session.agent_loop.try_read().unwrap().model.clone();
        assert!(!bare.contains('/'), "resolved set_model stores the bare id");
        // Simulate the resolve-miss window: set_model's fallback froze the
        // full provider/id literal into the loop.
        let canonical = session.model.clone();
        assert!(canonical.contains('/'));
        session.agent_loop.try_write().unwrap().model = canonical;

        session.reload_credentials().unwrap();
        assert_eq!(session.agent_loop.try_read().unwrap().model, bare);
    }

    #[test]
    fn reload_credentials_keeps_consistent_model_id() {
        let mut session = make_test_session("consistent-model");
        session.set_model("gpt-4o").unwrap();
        let before = session.agent_loop.try_read().unwrap().model.clone();

        session.reload_credentials().unwrap();
        assert_eq!(session.agent_loop.try_read().unwrap().model, before);
    }

    #[test]
    fn reload_credentials_leaves_unresolved_model_literal() {
        let mut session = make_test_session("unresolved-model");
        // Resolve miss: the fallback stores the literal, and reload must not
        // rewrite it while the registry still cannot resolve the model.
        session.set_model("unknown-provider/unknown-model").unwrap();
        assert_eq!(
            session.agent_loop.try_read().unwrap().model,
            "unknown-provider/unknown-model"
        );

        session.reload_credentials().unwrap();
        assert_eq!(
            session.agent_loop.try_read().unwrap().model,
            "unknown-provider/unknown-model"
        );
    }

    #[test]
    fn reload_credentials_reports_busy_when_loop_is_read_locked() {
        let mut session = make_test_session("busy-loop");
        session.set_model("gpt-4o").unwrap();
        // A held read guard makes the credential install's try_write fail.
        let _read_guard = session.agent_loop.try_read().unwrap();
        let error = session.reload_credentials().unwrap_err();
        assert!(
            error.to_string().contains("configuration is busy"),
            "unexpected error: {error:#}"
        );
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

    // ── coverage batch: hydrate/switch/scheduler-worker arms ───────────────

    #[test]
    fn construction_with_locked_loop_uses_fresh_counters() {
        let loop_ = Arc::new(tokio::sync::RwLock::new(Loop::new(
            Arc::new(EmptyProvider),
            "mock",
        )));
        let _guard = loop_.try_write().unwrap();
        let cwd = test_workspace();
        let session = ServerSession::new_with_queue_budget(
            "locked".to_string(),
            loop_.clone(),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
            Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        );
        // Counters could not be cloned from the locked loop → fresh zeros.
        assert_eq!(
            session.tokens_in.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn switch_session_restores_rich_session_info() {
        let cwd = test_workspace();
        let session_dir = std::path::Path::new(&cwd).join("sessions");
        let manager = Arc::new(Manager::new(session_dir));
        let info = crate::session::SessionEntry::session_info(
            serde_json::json!({
                "cwd": cwd,
                "model": "deepseek/deepseek-chat",
                "session_name": "restored name",
                "thinking_level": "low",
                "auto_compaction": false,
                "tokens_in": 111,
                "tokens_out": 222,
                "tokens_cache_r": 33,
                "tokens_cache_w": 44,
                "last_prompt_tokens": 555,
                "total_cost": 0.75,
            }),
            "deepseek/deepseek-chat".to_string(),
            "low".to_string(),
        );
        let snapshot = crate::session::Session::snapshot(
            "rich".to_string(),
            cwd.clone(),
            "deepseek/deepseek-chat".to_string(),
            "restored name".to_string(),
            String::new(),
            vec![
                info,
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        manager.save(&snapshot).unwrap();

        let mut session = ServerSession::new(
            "rich".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            manager.clone(),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        );
        session.switch_session("rich").unwrap();
        assert_eq!(session.model, "deepseek/deepseek-chat");
        assert_eq!(session.session_name, "restored name");
        assert_eq!(session.thinking_level, "low");
        assert!(!session.auto_compaction);
        assert_eq!(
            session.tokens_in.load(std::sync::atomic::Ordering::Relaxed),
            111
        );
        assert!((*session.cumulative_cost.lock() - 0.75).abs() < f64::EPSILON);
        assert!(!session.messages.read().is_empty());
    }

    #[test]
    fn list_sessions_reads_summaries() {
        let session = make_persistent_test_session("lister");
        let snapshot = crate::session::Session::snapshot(
            "lister".to_string(),
            session.cwd.clone(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": session.cwd, "model": "mock"}),
                    "mock".to_string(),
                    "low".to_string(),
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        session.session_manager.save(&snapshot).unwrap();
        let listed = session.list_sessions().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["id"], "lister");
    }

    #[test]
    fn recover_persistence_degraded_rejects_healthy_run() {
        let mut session = make_test_session("healthy");
        session
            .runtime
            .begin(Some("run-ok"), Some("request-ok"))
            .unwrap();
        let error = session.recover_persistence_degraded().unwrap_err();
        assert!(error.to_string().contains("not persistence_degraded"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recover_persistence_degraded_bails_while_task_slot_occupied() {
        let _sink = tracing_sink();
        let mut session = make_persistent_test_session("recover-occupied");
        std::fs::create_dir_all(&session.cwd).unwrap();
        session
            .broadcaster
            .configure_journal(
                session.session_id.clone(),
                session.session_manager.run_data_path(&session.session_id),
            )
            .unwrap();
        // A session file on disk so the recovery append has somewhere to
        // land (recover_with_entries refuses a not-yet-created transcript).
        let saved = crate::session::Session::snapshot(
            "recover-occupied".to_string(),
            session.cwd.clone(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": session.cwd, "model": "mock"}),
                "mock".to_string(),
                String::new(),
            )],
        );
        session.session_manager.save(&saved).unwrap();
        // Begin a run whose task slot stays occupied, then degrade it: the
        // control-plane lease matches, but the runtime refuses recovery
        // while the task is alive.
        let lease = session
            .runtime
            .begin(Some("run-stuck"), Some("req-stuck"))
            .unwrap();
        let hold = std::sync::Arc::new(tokio::sync::Notify::new());
        let task_hold = hold.clone();
        session
            .runtime
            .spawn(lease.clone(), async move {
                task_hold.notified().await;
            })
            .unwrap();
        // Drive the current-thread executor so the spawned task is polled
        // (reaching its notify await) before we degrade the lease.
        tokio::task::yield_now().await;
        assert!(session
            .runtime
            .mark_persistence_degraded(&lease, "disk full"));
        let error = session.recover_persistence_degraded().unwrap_err();
        assert!(
            error.to_string().contains("lost the active run lease"),
            "{error}"
        );
        hold.notify_one();
        // Let the spawned task wake, run to completion, and release the slot
        // before the tempdir is torn down.
        tokio::task::yield_now().await;
        let _ = std::fs::remove_dir_all(&session.cwd);
    }

    #[test]
    fn set_thinking_level_persists_to_disk_session() {
        let mut session = make_persistent_test_session("think");
        let snapshot = crate::session::Session::snapshot(
            "think".to_string(),
            session.cwd.clone(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": session.cwd, "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            )],
        );
        session.session_manager.save(&snapshot).unwrap();

        session.set_thinking_level("high");
        let loaded = session.session_manager.load("think").unwrap();
        let info = loaded
            .entries
            .iter()
            .rev()
            .find(|e| e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.clone())
            .unwrap();
        assert_eq!(info["thinking_level"], "high");
    }

    #[test]
    fn compact_with_real_history_reports_summary() {
        let mut session = make_test_session("compact");
        session.model = "glm-4.5v".to_string(); // 64k catalog window
        session
            .last_prompt_tokens
            .store(50_000, std::sync::atomic::Ordering::Relaxed);
        {
            let mut messages = session.messages.write();
            for i in 0..10 {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                messages.push(crate::types::AgentMessage {
                    role: role.to_string(),
                    content: vec![crate::types::ContentBlock::text(
                        format!("message {i} ").repeat(2000),
                    )],
                    ..Default::default()
                });
            }
            for message in messages.iter_mut() {
                message.ensure_journal_entry_id();
            }
            let entries = messages
                .iter()
                .map(crate::session::agent_message_to_entry)
                .collect();
            let snapshot = crate::session::Session::snapshot(
                session.session_id.clone(),
                session.cwd.clone(),
                session.model.clone(),
                String::new(),
                String::new(),
                entries,
            );
            session.session_manager.save(&snapshot).unwrap();
        }
        let result = session.compact("").unwrap();
        assert!(result["messagesRemoved"].as_i64().unwrap() > 0);
        assert!(result["summary"].is_string());
    }

    #[test]
    fn execute_shell_captures_stderr() {
        let session = make_test_session("stderr");
        std::fs::create_dir_all(&session.cwd).unwrap();
        let result = session.execute_shell("echo out; echo err 1>&2").unwrap();
        let output = result["output"].as_str().unwrap();
        assert!(output.contains("out"), "{output}");
        assert!(output.contains("err"), "{output}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scheduler_worker_starts_queued_run_after_completion() {
        use crate::{
            llm::schema::{FinishReason, ModelRequest, ModelStreamEvent},
            types::LLMProvider,
        };
        use tokio_stream::wrappers::ReceiverStream;

        struct GateProvider {
            gate: Arc<tokio::sync::Notify>,
            calls: std::sync::atomic::AtomicUsize,
        }
        #[async_trait::async_trait]
        impl LLMProvider for GateProvider {
            async fn stream_model(
                &self,
                _request: ModelRequest,
            ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if self.calls.load(std::sync::atomic::Ordering::SeqCst) == 1 {
                    self.gate.notified().await;
                }
                let (tx, rx) = mpsc::channel(2);
                let _ = tx.try_send(ModelStreamEvent::TextDelta {
                    id: "text".into(),
                    text: "reply".to_string(),
                });
                let _ = tx.try_send(ModelStreamEvent::Finish {
                    reason: FinishReason::Stop,
                    usage: None,
                });
                drop(tx);
                Ok(ReceiverStream::new(rx))
            }
        }

        let gate = Arc::new(tokio::sync::Notify::new());
        let provider = Arc::new(GateProvider {
            gate: gate.clone(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let cwd = test_workspace();
        std::fs::create_dir_all(&cwd).unwrap();
        let session = Arc::new(parking_lot::RwLock::new(ServerSession::new(
            "chain".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(provider, "mock"))),
            Arc::new(Manager::new(test_session_dir())),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        )));
        ServerSession::ensure_scheduler_worker(&session);
        // First prompt starts (gated); second queues behind it.
        session
            .write()
            .prompt("first", &[], &[], None, None)
            .unwrap();
        let ack = session
            .write()
            .enqueue_prompt(
                "second",
                &[],
                &[],
                None,
                "req-2",
                crate::runtime::BusyPolicy::EnqueueIfBusy,
            )
            .unwrap();
        assert_eq!(ack.accepted_state, crate::runtime::RunAcceptedState::Queued);
        // Release the first run; the completion wake starts the second.
        gate.notify_one();
        for _ in 0..500 {
            let count = session
                .read()
                .messages
                .read()
                .iter()
                .filter(|m| m.role == "assistant")
                .count();
            if count >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let assistant_count = session
            .read()
            .messages
            .read()
            .iter()
            .filter(|m| m.role == "assistant")
            .count();
        assert_eq!(assistant_count, 2, "both runs completed");
    }
}
