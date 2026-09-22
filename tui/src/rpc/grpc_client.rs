//! gRPC client — port of `tui/src/rpc/grpc-client.ts` `GrpcClient`.
//!
//! The TS client keeps a persistent `StreamEvents` subscription with reconnect
//! (1 s `tryConnect` polling, 3 consecutive-failure channel reset), a 10 s
//! heartbeat, and a 5 s first-data watchdog. This port reproduces all of it
//! with tonic + tokio:
//!
//!   - the stream is driven by a long-lived manager task spawned by the
//!     client (restarted on session change / reconnect)
//!   - parsed events are pushed into an `UnboundedSender<AgentEvent>` owned
//!     by the caller (the app loop)
//!   - connection-state changes are signalled through a `watch` channel
//!   - unary calls reuse the CLI port's per-call connect + deadline pattern
//!     (tonic `Endpoint::timeout`), which also makes the TS "recreate the
//!     channel after 3 failures" step a no-op — every call already gets a
//!     fresh channel.
//!
//! Like the TS client, transport failures and `success:false` responses
//! surface as plain `String` messages.

use crate::rpc::provider_types::{
    parse_providers_response, validate_provider_input, ProviderInfo, ProviderInput,
};
use crate::rpc::types::{
    AgentEvent, ModelInfo, ProjectedRunEvent, RpcSessionState, RunAck, SessionSummary,
};
use future_rpc::proto::future_agent_client::FutureAgentClient;
use future_rpc::proto::{Attachment, RpcCommand, StreamEvent, StreamRequest};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch, Notify};
use uuid::Uuid;

/// Default gRPC deadline (seconds) for unary calls (grpc-client.ts
/// `GRPC_DEADLINE_SEC`).
const GRPC_DEADLINE_SEC: u64 = 30;
/// `tryConnect` timeout (seconds).
const TRY_CONNECT_TIMEOUT_SEC: u64 = 3;
/// First-data watchdog: if the stream delivers nothing within 5 s the
/// underlying channel is likely stuck — cancel and reconnect.
const CONNECT_WATCHDOG_MS: u64 = 5_000;
/// Reconnect poll interval.
const RECONNECT_POLL_MS: u64 = 1_000;
/// Heartbeat interval (silent-disconnection detection). Tests run it fast
/// so the dead-agent path is exercisable without multi-second waits.
#[cfg(not(test))]
const HEARTBEAT_MS: u64 = 10_000;
#[cfg(test)]
const HEARTBEAT_MS: u64 = 50;
/// How long `call` waits for the event stream to deliver its first frame.
const CALL_CONNECT_WAIT_MS: u64 = 5_000;

/// `String(Date.now())` — millisecond epoch, used as the request correlation id.
fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

/// `crypto.randomUUID().replaceAll("-", "")` — hex uuid without dashes.
pub(crate) fn uuid_hex() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Mirrors the TS `isTransport` check on the error message.
fn is_transport_error(msg: &str) -> bool {
    msg.contains("transport")
        || msg.contains("14 UNAVAILABLE")
        || msg.contains("Connect Failed")
        || msg.contains("ECONNREFUSED")
}

/// Explicit TCP override, otherwise automatic per-user local IPC discovery.
pub fn grpc_addr() -> String {
    std::env::var("FUTURE_AGENT_GRPC_ADDR")
        .unwrap_or_else(|_| future_rpc::transport::AUTO_ENDPOINT.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunStatus {
    Queued,
    Running,
    Terminal,
}

/// Client state shared between the app loop and the stream manager task.
struct ClientState {
    connected: bool,
    current_session_id: String,
    active_run_id: Option<String>,
    /// The most recent run of this session, kept after the run ends: the
    /// run-scoped reads (`list_tool_calls` / `get_tool_output`) are asked for
    /// exactly when a run has finished and `active_run_id` is already `None`.
    last_run_id: Option<String>,
    runs: HashMap<String, RunStatus>,
    agent_instance_id: Option<String>,
    lost_queued_run_ids: Vec<String>,
}

struct Inner {
    addr: String,
    state: Mutex<ClientState>,
    /// Parsed stream events — consumed by the app loop.
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    /// Connection-change notifications (mirrors `notifyConnectionChange`).
    conn_tx: watch::Sender<bool>,
    /// Poke counter — bumped by `connect_events`/session changes to wake the
    /// stream manager (wakes it even when the value is unchanged, so a plain
    /// `Notify` cannot drop the wakeup).
    poke_count: AtomicU64,
    poke_notify: Notify,
    #[cfg(test)]
    before_empty_wait: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    /// Permanent stop (disconnect).
    stop: AtomicBool,
    stop_notify: Notify,
}

/// gRPC client for FutureAgent (port of `GrpcClient`).
///
/// The client is shared between the app loop and spawned tasks via `Arc`;
/// the event stream receiver and connection-change watch are split out by
/// `new()` (a single-consumer receiver cannot live behind a shared `Arc`).
pub struct GrpcClient {
    inner: Arc<Inner>,
    no_context_files: bool,
}

impl GrpcClient {
    /// Create a client bound to `addr` (default `localhost:50051`). Returns
    /// the client plus its event stream receiver and connection-change watch
    /// (initial value `false`), both meant for the app's event loop.
    pub fn new(
        addr: &str,
    ) -> (
        GrpcClient,
        mpsc::UnboundedReceiver<AgentEvent>,
        watch::Receiver<bool>,
    ) {
        let (event_tx, events) = mpsc::unbounded_channel();
        let (conn_tx, connection_changes) = watch::channel(false);
        let inner = Arc::new(Inner {
            addr: addr.to_string(),
            state: Mutex::new(ClientState {
                connected: false,
                current_session_id: String::new(),
                active_run_id: None,
                last_run_id: None,
                runs: HashMap::new(),
                agent_instance_id: None,
                lost_queued_run_ids: Vec::new(),
            }),
            event_tx,
            conn_tx,
            poke_count: AtomicU64::new(0),
            poke_notify: Notify::new(),
            #[cfg(test)]
            before_empty_wait: Mutex::new(None),
            stop: AtomicBool::new(false),
            stop_notify: Notify::new(),
        });

        // Stream manager: subscribes to the current session's event stream,
        // reconnects on failure (1 s tryConnect polling), restarts when the
        // session changes or `connect_events` is called.
        spawn_stream_manager(inner.clone());
        // Heartbeat: detects silent disconnections (agent SIGKILL'd).
        spawn_heartbeat(inner.clone());

        (
            GrpcClient {
                inner,
                no_context_files: false,
            },
            events,
            connection_changes,
        )
    }

    /// Keep the CLI opt-out across /new, switch, fork and agent reconnects.
    pub fn with_no_context_files(mut self, disabled: bool) -> Self {
        self.no_context_files = disabled;
        self
    }

    // ─── Connection state ──────────────────────────────────────────────

    /// `isConnected()`.
    pub fn is_connected(&self) -> bool {
        self.inner.state.lock().connected
    }

    /// `getCurrentSessionId()`.
    pub fn get_current_session_id(&self) -> String {
        self.inner.state.lock().current_session_id.clone()
    }

    /// `setCurrentSessionId(sessionId)` — clears the run bookkeeping and
    /// wakes the stream manager (session changed → resubscribe).
    pub fn set_current_session_id(&self, session_id: &str) {
        {
            let mut st = self.inner.state.lock();
            st.current_session_id = session_id.to_string();
            st.active_run_id = None;
            st.last_run_id = None;
            st.runs.clear();
        }
        self.poke();
    }

    /// `connectEvents()` — cancel the existing stream and resubscribe.
    pub fn connect_events(&self) {
        self.poke();
    }

    /// `disconnect()` — stop the stream + heartbeat and mark disconnected.
    pub fn disconnect(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.stop_notify.notify_waiters();
        self.inner.state.lock().connected = false;
    }

    fn poke(&self) {
        self.inner.poke_count.fetch_add(1, Ordering::SeqCst);
        self.inner.poke_notify.notify_waiters();
    }

    // ─── Event streaming ───────────────────────────────────────────────

    /// Lightweight connectivity check (`tryConnect`): `list_models` unary
    /// with a 3 s deadline. Returns true if the agent is reachable.
    pub async fn try_connect(&self) -> bool {
        let cmd = RpcCommand {
            id: now_id(),
            r#type: "list_models".to_string(),
            ..Default::default()
        };
        execute_unary(&self.inner.addr, cmd, TRY_CONNECT_TIMEOUT_SEC)
            .await
            .is_ok()
    }

    /// `takeLostQueuedRunIds()` — queued work lost across an agent restart.
    pub fn take_lost_queued_run_ids(&self) -> Vec<String> {
        let mut st = self.inner.state.lock();
        std::mem::take(&mut st.lost_queued_run_ids)
    }

    /// `hasRunningRun()`.
    pub fn has_running_run(&self) -> bool {
        self.inner
            .state
            .lock()
            .runs
            .values()
            .any(|s| *s == RunStatus::Running)
    }

    // ─── RPC call helper ───────────────────────────────────────────────

    /// `call(type, cmd)` — injects `id`/`type`/`sessionId` (from the current
    /// session) into `cmd`, waits up to 5 s for the stream to connect when
    /// disconnected, executes with a 30 s deadline, and parses the JSON data
    /// payload (typed-first decode, `Null` when empty or non-JSON).
    async fn call(&self, r#type: &str, mut cmd: RpcCommand) -> Result<Value, String> {
        // Wait for connection if not yet connected (first call or
        // reconnecting): await the event stream's first frame, bounded by 5 s.
        // With no session yet (startup), skip the wait — the TS
        // `Promise.race([connectPromise, timeout])` resolves immediately when
        // `connectPromise` is null (connectEvents returns early on an empty
        // session id).
        if !self.is_connected() && !self.inner.state.lock().current_session_id.is_empty() {
            self.wait_connected(CALL_CONNECT_WAIT_MS).await;
        }

        {
            let st = self.inner.state.lock();
            cmd.id = now_id();
            cmd.r#type = r#type.to_string();
            // TS: `sessionId: this.currentSessionId || undefined` spread
            // BEFORE `...cmd` — an explicit sessionId in cmd wins.
            // `new_session` must stay session-less: the agent treats a
            // non-empty session_id as "create THIS id" and restores its
            // persisted entries, so inheriting the current id would hand
            // back the same session with its full history (TS expressed
            // this as `sessionId: undefined` overriding the spread).
            if cmd.session_id.is_empty()
                && !st.current_session_id.is_empty()
                && r#type != "new_session"
            {
                cmd.session_id = st.current_session_id.clone();
            }
        }

        // Apply the opt-out to the exact session this request addresses, even
        // after /new or reconnect. Fail closed: an older/unavailable agent must
        // not receive a prompt with project instructions unexpectedly enabled.
        if self.no_context_files
            && !cmd.session_id.is_empty()
            && matches!(r#type, "prompt" | "get_state" | "reload_config")
        {
            execute_unary(
                &self.inner.addr,
                RpcCommand {
                    id: now_id(),
                    r#type: "set_context_files".to_string(),
                    session_id: cmd.session_id.clone(),
                    enabled: false,
                    ..Default::default()
                },
                GRPC_DEADLINE_SEC,
            )
            .await?;
        }
        let result = execute_unary(&self.inner.addr, cmd, GRPC_DEADLINE_SEC).await;
        if let Err(ref err) = result {
            // On transport error, trigger reconnect so the stream comes back.
            // Don't retry the call — for non-idempotent commands like 'prompt'
            // the request may have already reached the agent. When the stream
            // still reports connected, a transient unary failure must NOT tear
            // down the working stream (TS comment).
            if is_transport_error(err) && !self.is_connected() {
                self.connect_events();
            }
        }
        result
    }

    /// Wait (bounded) for the event stream to deliver its first frame.
    async fn wait_connected(&self, max_ms: u64) {
        let mut rx = self.inner.conn_tx.subscribe();
        // wait_for returns on the first `true`; a closed channel (teardown)
        // errors out of it — either way the timeout caps the wait.
        let _ = tokio::time::timeout(
            Duration::from_millis(max_ms),
            rx.wait_for(|connected| *connected),
        )
        .await;
    }

    // ─── Session management ────────────────────────────────────────────

    /// `newSession(opts?)` — `new_session` with `createdBy: "tui"`; the
    /// sessionId field is left empty so the agent generates a fresh ID.
    pub async fn new_session(
        &self,
        cwd: Option<&str>,
        model_id: Option<&str>,
        level: Option<&str>,
    ) -> Result<Value, String> {
        let default_cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let cmd = RpcCommand {
            cwd: cwd.unwrap_or(&default_cwd).to_string(),
            model_id: model_id.unwrap_or("").to_string(),
            level: level.unwrap_or("").to_string(),
            created_by: "tui".to_string(),
            ..Default::default()
        };
        let result = self.call("new_session", cmd).await?;
        if let Some(sid) = result.get("sessionId").and_then(Value::as_str) {
            self.set_current_session_id(sid);
            self.connect_events();
        }
        Ok(result)
    }

    /// `switchSession(sessionId)`.
    pub async fn switch_session(&self, session_id: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            session_id: session_id.to_string(),
            ..Default::default()
        };
        let result = self.call("switch_session", cmd).await?;
        let cancelled = result
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !cancelled {
            self.set_current_session_id(session_id);
            self.connect_events();
        }
        Ok(result)
    }

    /// `fork(entryId)`.
    pub async fn fork(&self, entry_id: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            entry_id: entry_id.to_string(),
            ..Default::default()
        };
        let result = self.call("fork", cmd).await?;
        if let Some(sid) = result.get("sessionId").and_then(Value::as_str) {
            self.set_current_session_id(sid);
            self.connect_events();
        }
        Ok(result)
    }

    /// `clone()`.
    pub async fn clone_session(&self) -> Result<Value, String> {
        let result = self.call("clone", RpcCommand::default()).await?;
        if let Some(sid) = result.get("sessionId").and_then(Value::as_str) {
            self.set_current_session_id(sid);
            self.connect_events();
        }
        Ok(result)
    }

    /// `getForkMessages()` — `{messages: [...]}`.
    pub async fn get_fork_messages(&self) -> Result<Value, String> {
        self.call("get_fork_messages", RpcCommand::default()).await
    }

    /// `setSessionName(name)`.
    pub async fn set_session_name(&self, name: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            name: name.to_string(),
            ..Default::default()
        };
        self.call("set_session_name", cmd).await?;
        Ok(())
    }

    /// `listSessions()` — `{sessions: [SessionSummary, ...]}`.
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        let resp = self.call("list_sessions", RpcCommand::default()).await?;
        let sessions = resp
            .get("sessions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let parsed = sessions
            .iter()
            .filter_map(|s| serde_json::from_value::<SessionSummary>(s.clone()).ok())
            .collect::<Vec<_>>();
        Ok(parsed)
    }

    // ─── Core RPC methods ──────────────────────────────────────────────

    /// `prompt(message, images?, busyPolicy)` — generates `requestedRunId`
    /// and `clientRequestId` like the TS client and records the ack's run
    /// status.
    ///
    /// `attachments` are the images the draft references: only their paths
    /// cross the wire (the agent reads and encodes the bytes itself), and the
    /// agent turns each one into an `image_url` block when the active model
    /// accepts image input — otherwise it lists the path in the prompt so the
    /// model can still read the file with its own tools.
    pub async fn prompt(
        &self,
        message: &str,
        busy_policy: &str,
        attachments: Vec<Attachment>,
    ) -> Result<RunAck, String> {
        let request_id = uuid_hex();
        let cmd = RpcCommand {
            message: message.to_string(),
            requested_run_id: format!("run_{}", uuid_hex()),
            client_request_id: format!("request_{request_id}"),
            busy_policy: busy_policy.to_string(),
            attachments,
            ..Default::default()
        };
        let resp = self.call("prompt", cmd).await?;
        let ack: RunAck = serde_json::from_value(resp).map_err(|e| e.to_string())?;
        let mut st = self.inner.state.lock();
        st.last_run_id = Some(ack.run_id.clone());
        if ack.accepted_state == "running" {
            st.active_run_id = Some(ack.run_id.clone());
            st.runs.insert(ack.run_id.clone(), RunStatus::Running);
        } else if ack.accepted_state == "queued" {
            st.runs.insert(ack.run_id.clone(), RunStatus::Queued);
        }
        Ok(ack)
    }

    /// `abort()`.
    pub async fn abort(&self) -> Result<(), String> {
        let run_id = self.inner.state.lock().active_run_id.clone();
        let cmd = RpcCommand {
            run_id: run_id.unwrap_or_default(),
            ..Default::default()
        };
        self.call("abort", cmd).await?;
        Ok(())
    }

    /// `cancelQueuedRun(runId)`.
    pub async fn cancel_queued_run(&self, run_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            run_id: run_id.to_string(),
            ..Default::default()
        };
        self.call("cancel_queued_run", cmd).await?;
        self.inner.state.lock().runs.remove(run_id);
        Ok(())
    }

    /// `getState()` — typed `RpcSessionState`; updates the run bookkeeping
    /// and detects agent restarts (lost queued runs).
    pub async fn get_state(&self) -> Result<RpcSessionState, String> {
        let resp = self.call("get_state", RpcCommand::default()).await?;
        let state: RpcSessionState = serde_json::from_value(resp).map_err(|e| e.to_string())?;

        let mut st = self.inner.state.lock();
        if let (Some(prev), Some(cur)) = (&st.agent_instance_id, &state.agent_instance_id) {
            if prev != cur {
                let lost: Vec<String> = st
                    .runs
                    .iter()
                    .filter(|(_, status)| **status == RunStatus::Queued)
                    .map(|(run_id, _)| run_id.clone())
                    .collect();
                st.lost_queued_run_ids.extend(lost);
            }
        }
        if let Some(id) = &state.agent_instance_id {
            st.agent_instance_id = Some(id.clone());
        }
        st.runs.clear();
        if let Some(active) = &state.active_run {
            st.active_run_id = Some(active.run_id.clone());
            st.last_run_id = Some(active.run_id.clone());
            st.runs.insert(active.run_id.clone(), RunStatus::Running);
        } else {
            st.active_run_id = None;
        }
        for queued in &state.queued_runs {
            st.runs.insert(queued.run_id.clone(), RunStatus::Queued);
        }
        drop(st);
        Ok(state)
    }

    /// `getMessages()` — `{messages: [...]}` (session entry reconstruction).
    pub async fn get_messages(&self) -> Result<Value, String> {
        self.call("get_messages", RpcCommand::default()).await
    }

    /// `setModel(modelId)`.
    pub async fn set_model(&self, model_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            model_id: model_id.to_string(),
            ..Default::default()
        };
        self.call("set_model", cmd).await?;
        Ok(())
    }

    /// `cycleModel()` — `{model, thinkingLevel, isScoped} | null`.
    pub async fn cycle_model(&self) -> Result<Value, String> {
        self.call("cycle_model", RpcCommand::default()).await
    }

    /// `listModels()` — `list_models` response with `models: ModelInfo[]`.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, String> {
        let resp = self.call("list_models", RpcCommand::default()).await?;
        let models = resp
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let parsed = models
            .iter()
            .filter_map(|m| serde_json::from_value::<ModelInfo>(m.clone()).ok())
            .collect::<Vec<_>>();
        Ok(parsed)
    }

    /// `setThinkingLevel(level)`.
    pub async fn set_thinking_level(&self, level: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            level: level.to_string(),
            ..Default::default()
        };
        self.call("set_thinking_level", cmd).await?;
        Ok(())
    }

    /// `cycleThinkingLevel()` — `{level} | null`.
    pub async fn cycle_thinking_level(&self) -> Result<Value, String> {
        self.call("cycle_thinking_level", RpcCommand::default())
            .await
    }

    /// `compact(customInstructions?)` — returns the result string.
    pub async fn compact(&self, custom_instructions: Option<&str>) -> Result<String, String> {
        let cmd = RpcCommand {
            custom_instructions: custom_instructions.unwrap_or("").to_string(),
            ..Default::default()
        };
        let resp = self.call("compact", cmd).await?;
        Ok(resp.as_str().unwrap_or("").to_string())
    }

    /// `setCwd(cwd)`.
    pub async fn set_cwd(&self, cwd: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            cwd: cwd.to_string(),
            ..Default::default()
        };
        self.call("set_cwd", cmd).await?;
        Ok(())
    }

    /// `approvalDecision(requestId, approved, note?)`.
    pub async fn approval_decision(
        &self,
        request_id: &str,
        approved: bool,
        note: &str,
    ) -> Result<(), String> {
        let cmd = RpcCommand {
            mode: if approved { "approved" } else { "rejected" }.to_string(),
            message: note.to_string(),
            entry_id: request_id.to_string(),
            ..Default::default()
        };
        self.call("approval_decision", cmd).await?;
        Ok(())
    }

    /// `setPermissionLevel(level)`.
    pub async fn set_permission_level(&self, level: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            level: level.to_string(),
            ..Default::default()
        };
        self.call("set_permission_level", cmd).await?;
        Ok(())
    }

    // ─── Sandbox (tier + availability probe) ────────────────────────────

    /// `probeSandbox()` — `{available, code, backend, path?, version?}` for the
    /// host's OS sandbox. Read-only: the agent caches the answer per process,
    /// so a client may call it once and keep the result.
    pub async fn probe_sandbox(&self) -> Result<Value, String> {
        self.call("probe_sandbox", RpcCommand::default()).await
    }

    /// `probeWindowsSandbox()` — the Windows write-protection probe. Only
    /// meaningful on a Windows host (elsewhere the agent reports the host
    /// probe's diagnostic).
    pub async fn probe_windows_sandbox(&self) -> Result<Value, String> {
        self.call("probe_windows_sandbox", RpcCommand::default())
            .await
    }

    /// `setSandboxPolicy(tier)` — `off` | `manual` | `sandbox`.
    ///
    /// Returns the agent's *own* summary rather than `()`: the agent answers
    /// `tier:"manual"` with `requestedTier:"sandbox"` when the probe found no
    /// usable backend, and reporting that as an applied request would lie to
    /// the user. Callers must read `tier`/`requestedTier` from the response.
    pub async fn set_sandbox_policy(&self, tier: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            sandbox_policy: Some(future_rpc::proto::SandboxPolicy {
                tier: tier.to_string(),
            }),
            ..Default::default()
        };
        self.call("set_sandbox_policy", cmd).await
    }

    // ─── Skills catalogue ──────────────────────────────────────────────

    /// `getCommands()` — `{commands:[{name, description, nameZh?, descriptionZh?,
    /// source}]}`. Rows with `source == "skill"` are the discovered skills.
    pub async fn get_commands(&self) -> Result<Value, String> {
        self.call("get_commands", RpcCommand::default()).await
    }

    /// `refreshSkills()` — re-scan the skill directories. The agent's payload
    /// is snake_case (`{refreshed, skills, skills_count}`), unlike most commands.
    pub async fn refresh_skills(&self) -> Result<Value, String> {
        self.call("refresh_skills", RpcCommand::default()).await
    }

    /// `reloadConfig()` — `{skills, contextFiles}`.
    pub async fn reload_config(&self) -> Result<Value, String> {
        self.call("reload_config", RpcCommand::default()).await
    }

    // ─── Provider / auth configuration (sessionless) ────────────────────

    /// `listProviders()` — the agent's provider view (`{builtin, custom}`)
    /// flattened to built-ins first. Entries the parser cannot use are skipped
    /// rather than failing the list.
    pub async fn list_providers(&self) -> Result<Vec<ProviderInfo>, String> {
        let resp = self.call("list_providers", RpcCommand::default()).await?;
        Ok(parse_providers_response(&resp))
    }

    /// `upsertProvider(input)` — create/update a **custom** provider (plus its
    /// optional API key). Validated client-side first so the form gets the same
    /// message without a round-trip; the agent stays authoritative.
    ///
    /// Built-in providers cannot be redefined (the agent rejects it) — their
    /// key/URL go through [`Self::set_auth_key`] instead.
    pub async fn upsert_provider(&self, provider: &ProviderInput) -> Result<(), String> {
        validate_provider_input(provider)?;
        let cmd = RpcCommand {
            provider_config: Some(provider.to_proto()),
            ..Default::default()
        };
        self.call("upsert_provider", cmd).await?;
        Ok(())
    }

    /// `deleteProvider(id)` — removes the models.json entry and the auth.json
    /// entry. Built-in provider ids are refused by the agent.
    pub async fn delete_provider(&self, id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            provider_config: Some(future_rpc::proto::ProviderUpsert {
                id: id.trim().to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.call("delete_provider", cmd).await?;
        Ok(())
    }

    /// `setAuthKey(provider, key)` — store `key` for `provider`, or clear the
    /// stored key when `key` is `None`/blank. Applies to built-in providers too
    /// (this is how the Future sign-in key is set or removed).
    pub async fn set_auth_key(&self, provider: &str, key: Option<&str>) -> Result<(), String> {
        let key = key.filter(|key| !key.trim().is_empty());
        let cmd = RpcCommand {
            auth_update: Some(future_rpc::proto::AuthUpdate {
                provider: provider.trim().to_string(),
                key: key.unwrap_or("").to_string(),
                clear_key: key.is_none(),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.call("set_auth", cmd).await?;
        Ok(())
    }

    /// `reloadAuth()` — rebuild the agent's provider registry from disk.
    pub async fn reload_auth(&self) -> Result<(), String> {
        self.call("reload_auth", RpcCommand::default()).await?;
        Ok(())
    }

    /// `syncFutureModels()` — fetch the Future catalogue; returns
    /// `{synced, modelCount, revision}`.
    pub async fn sync_future_models(&self) -> Result<Value, String> {
        self.call("sync_future_models", RpcCommand::default()).await
    }

    /// `setDefaultModel(modelId)` — persist the global default model.
    pub async fn set_default_model(&self, model_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            model_id: model_id.to_string(),
            ..Default::default()
        };
        self.call("set_default_model", cmd).await?;
        Ok(())
    }

    /// `getAgentInfo()` — `{version, agentInstanceId, skillsCount}`.
    pub async fn get_agent_info(&self) -> Result<Value, String> {
        self.call("get_agent_info", RpcCommand::default()).await
    }

    // ─── Session inspection (session-scoped) ───────────────────────────

    /// `getSessionStats()` — message/tool/token counters plus cost.
    pub async fn get_session_stats(&self) -> Result<Value, String> {
        self.call("get_session_stats", RpcCommand::default()).await
    }

    /// The run the run-scoped reads address: the live run, else the most
    /// recent one of this session.
    fn inspection_run_id(&self) -> Option<String> {
        let st = self.inner.state.lock();
        st.active_run_id.clone().or_else(|| st.last_run_id.clone())
    }

    /// `listToolCalls()` — stored tool calls of the current (or last) run.
    /// Errors when the session has no run yet: the agent requires a run id.
    pub async fn list_tool_calls(&self) -> Result<Value, String> {
        let run_id = self
            .inspection_run_id()
            .ok_or_else(|| "no run to list tool calls for".to_string())?;
        let cmd = RpcCommand {
            run_id,
            ..Default::default()
        };
        self.call("list_tool_calls", cmd).await
    }

    /// `getToolOutput(toolCallId)` — the full stored output of one tool call of
    /// the current (or last) run.
    pub async fn get_tool_output(&self, tool_call_id: &str) -> Result<Value, String> {
        let run_id = self
            .inspection_run_id()
            .ok_or_else(|| "no run to read tool output for".to_string())?;
        let cmd = RpcCommand {
            run_id,
            tool_call_id: Some(tool_call_id.to_string()),
            ..Default::default()
        };
        self.call("get_tool_output", cmd).await
    }

    /// `searchSessionHistory(query, limit)` — search the persisted session's
    /// original visible history (`message` carries the literal query).
    pub async fn search_session_history(&self, query: &str, limit: usize) -> Result<Value, String> {
        let cmd = RpcCommand {
            message: query.to_string(),
            limit: Some(limit as i64),
            ..Default::default()
        };
        self.call("search_session_history", cmd).await
    }

    /// `deleteSession(sessionId)` — delete one session (its record and file).
    /// The id is explicit so the caller never deletes "whatever is current" by
    /// accident.
    pub async fn delete_session(&self, session_id: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            session_id: session_id.to_string(),
            ..Default::default()
        };
        self.call("delete_session", cmd).await
    }

    /// `generateSessionTitle(mode)` — a model-suggested title for the current
    /// session. `mode` is the locale the agent's prompt is written in
    /// (`"zh"` | `"en"`; anything else is an agent-side error). The agent never
    /// persists the suggestion — apply it with [`Self::set_session_name`].
    pub async fn generate_session_title(&self, mode: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            mode: mode.to_string(),
            ..Default::default()
        };
        self.call("generate_session_title", cmd).await
    }

    /// `getRuntimeMetrics()` — the session's live runtime counters.
    pub async fn get_runtime_metrics(&self) -> Result<Value, String> {
        self.call("get_runtime_metrics", RpcCommand::default())
            .await
    }

    /// `getRunSnapshot(runId)` — the projection snapshot of one run.
    pub async fn get_run_snapshot(&self, run_id: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            run_id: run_id.to_string(),
            ..Default::default()
        };
        self.call("get_run_snapshot", cmd).await
    }

    /// The run `/snapshot` addresses: the live run, else the most recent one.
    /// Errors when the session has no run yet (the agent requires a run id).
    pub fn snapshot_run_id(&self) -> Result<String, String> {
        self.inspection_run_id()
            .ok_or_else(|| "no run to snapshot yet".to_string())
    }

    // ─── Prompt steering + session lifecycle ───────────────────────────

    /// `setContextFiles(enabled)` — whether the agent loads the workspace's
    /// project instructions (AGENTS.md, …) into the prompt.
    pub async fn set_context_files(&self, enabled: bool) -> Result<(), String> {
        let cmd = RpcCommand {
            enabled,
            ..Default::default()
        };
        self.call("set_context_files", cmd).await?;
        Ok(())
    }

    /// `shell(command)` — run one command through the agent's shell (the same
    /// `bash -c` / PowerShell contract as the shell tool, in the session cwd).
    /// `timeout_ms == 0` selects the agent's default.
    pub async fn shell(&self, command: &str, timeout_ms: u64) -> Result<Value, String> {
        let cmd = RpcCommand {
            command: command.to_string(),
            shell_timeout_ms: timeout_ms,
            ..Default::default()
        };
        self.call("shell", cmd).await
    }

    // ─── Session settings ──────────────────────────────────────────────

    /// `setAutoCompaction(enabled)`.
    pub async fn set_auto_compaction(&self, enabled: bool) -> Result<(), String> {
        let cmd = RpcCommand {
            enabled,
            ..Default::default()
        };
        self.call("set_auto_compaction", cmd).await?;
        Ok(())
    }

    /// `setAutoRetry(enabled)`.
    pub async fn set_auto_retry(&self, enabled: bool) -> Result<(), String> {
        let cmd = RpcCommand {
            enabled,
            ..Default::default()
        };
        self.call("set_auto_retry", cmd).await?;
        Ok(())
    }

    /// `setTools(tools)` — replace the session's enabled tool set.
    pub async fn set_tools(&self, tools: &[String]) -> Result<(), String> {
        let cmd = RpcCommand {
            tools: tools.to_vec(),
            ..Default::default()
        };
        self.call("set_tools", cmd).await?;
        Ok(())
    }

    /// `disableTools()` — turn every tool off for this session.
    pub async fn disable_tools(&self) -> Result<(), String> {
        self.call("disable_tools", RpcCommand::default()).await?;
        Ok(())
    }

    /// `exportHtml()` — write the session to an HTML file, `{path}`.
    pub async fn export_html(&self) -> Result<Value, String> {
        self.call("export_html", RpcCommand::default()).await
    }
}

// ─── Unary RPC ─────────────────────────────────────────────────────────────

/// One-shot `ExecuteCommand` with a deadline (mirrors the CLI port; the TS
/// client reuses a channel, per-call connect is equivalent for our use).
async fn execute_unary(addr: &str, cmd: RpcCommand, timeout_secs: u64) -> Result<Value, String> {
    let connected = future_rpc::transport::connect_channel(
        Some(addr),
        Duration::from_secs(timeout_secs.min(5)),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut client = FutureAgentClient::new(connected.channel);
    let response = client
        .execute_command(future_rpc::command_policy::request_with_timeout(cmd))
        .await
        .map_err(|status| {
            let msg = status.message();
            if msg.is_empty() {
                status.to_string()
            } else {
                msg.to_string()
            }
        })?
        .into_inner();

    if !response.success {
        return Err(if response.error.is_empty() {
            "unknown error".to_string()
        } else {
            response.error
        });
    }

    // Typed-first decode, shared with the CLI/GUI clients: the typed
    // `payload` wins, then the JSON `data` string. Non-JSON `data` yields
    // `Null` rather than a string passthrough.
    Ok(future_rpc::decode::response_data(&response))
}

// ─── Event stream manager ──────────────────────────────────────────────────

/// Build the `AgentEvent` the TS client pushes: the envelope keys in order,
/// then the parsed `data` spread over them (data wins on key collisions,
/// envelope keys keep their position).
fn parse_stream_event(event: &StreamEvent, raw_data: &Map<String, Value>) -> AgentEvent {
    let mut data = Map::new();
    data.insert(
        "type".to_string(),
        Value::String(if event.r#type.is_empty() {
            "message".to_string()
        } else {
            event.r#type.clone()
        }),
    );
    data.insert(
        "sessionId".to_string(),
        Value::String(event.session_id.clone()),
    );
    data.insert("runId".to_string(), Value::String(event.run_id.clone()));
    data.insert("epoch".to_string(), json!(event.epoch));
    data.insert("idx".to_string(), json!(event.idx));
    data.insert("eventId".to_string(), Value::String(event.event_id.clone()));
    data.insert(
        "timestamp".to_string(),
        Value::String(event.timestamp.clone()),
    );
    data.insert(
        "projectionSnapshot".to_string(),
        Value::Bool(event.projection_snapshot),
    );
    data.insert("snapshotCursor".to_string(), json!(event.snapshot_cursor));
    data.insert(
        "snapshotEvents".to_string(),
        Value::Array(
            event
                .snapshot_events
                .iter()
                .map(|e| {
                    json!({
                        "type": e.r#type,
                        "data": e.data,
                        "idx": e.idx,
                    })
                })
                .collect(),
        ),
    );
    for (k, v) in raw_data {
        data.insert(k.clone(), v.clone());
    }

    AgentEvent {
        r#type: if event.r#type.is_empty() {
            "message".to_string()
        } else {
            event.r#type.clone()
        },
        session_id: if event.session_id.is_empty() {
            None
        } else {
            Some(event.session_id.clone())
        },
        run_id: if event.run_id.is_empty() {
            None
        } else {
            Some(event.run_id.clone())
        },
        epoch: event.epoch,
        idx: event.idx,
        event_id: if event.event_id.is_empty() {
            None
        } else {
            Some(event.event_id.clone())
        },
        timestamp: if event.timestamp.is_empty() {
            None
        } else {
            Some(event.timestamp.clone())
        },
        projection_snapshot: event.projection_snapshot,
        snapshot_cursor: event.snapshot_cursor,
        snapshot_events: event
            .snapshot_events
            .iter()
            .map(|e| ProjectedRunEvent {
                r#type: e.r#type.clone(),
                data: e.data.clone(),
                idx: e.idx,
            })
            .collect(),
        data: Value::Object(data),
    }
}

/// Long-lived task: subscribe to the current session's event stream; on
/// stream end/error/watchdog, mark disconnected and poll `tryConnect` every
/// 1 s until the agent answers (then resubscribe). A poke (session change or
/// explicit `connect_events`) aborts the current subscription and restarts.
fn spawn_stream_manager(inner: Arc<Inner>) {
    tokio::spawn(async move {
        loop {
            // Register before reading state: notify_waiters does not retain a
            // permit for a future created after the notification.
            let poked = inner.poke_notify.notified();
            tokio::pin!(poked);
            poked.as_mut().enable();
            if inner.stop.load(Ordering::SeqCst) {
                return;
            }

            let session = inner.state.lock().current_session_id.clone();
            if session.is_empty() {
                #[cfg(test)]
                {
                    let barrier = inner.before_empty_wait.lock().take();
                    if let Some(barrier) = barrier {
                        barrier.wait().await;
                        barrier.wait().await;
                    }
                }
                // Never subscribe without a session ID — an empty session_id
                // may leak events from ALL sessions (TS comment). Wait for a
                // poke or stop.
                tokio::select! {
                    _ = &mut poked => {}
                    _ = inner.stop_notify.notified() => {
                        if inner.stop.load(Ordering::SeqCst) { return; }
                    }
                }
                continue;
            }

            // Subscribe until the stream ends, errors, the 5 s first-data
            // watchdog fires, or the session changes.
            match subscribe_stream(&inner, &session).await {
                StreamExit::Poked => {
                    // Silent resubscribe (session change / connectEvents):
                    // like TS, the cancelled stale stream never notifies
                    // false and `connected` stays true. Loop back to the top
                    // to subscribe with the current session immediately.
                    if inner.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    continue;
                }
                StreamExit::Lost => {}
            }
            if inner.stop.load(Ordering::SeqCst) {
                return;
            }

            let was_connected = {
                let mut st = inner.state.lock();
                let was = st.connected;
                st.connected = false;
                was
            };
            if was_connected {
                let _ = inner.conn_tx.send(false);
            }

            // Reconnect poll: tryConnect every 1 s (the TS reconnect loop —
            // polling via unary RPC instead of blindly re-subscribing, since a
            // dead channel returns a stream that never emits data/error/end).
            loop {
                if inner.stop.load(Ordering::SeqCst) {
                    return;
                }
                let session_now = inner.state.lock().current_session_id.clone();
                if session_now != session {
                    break; // session changed — resubscribe at the top
                }
                tokio::select! {
                    _ = inner.poke_notify.notified() => {
                        // Session change or explicit connect_events: try the
                        // stream right away.
                    }
                    _ = inner.stop_notify.notified() => {
                        if inner.stop.load(Ordering::SeqCst) { return; }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(RECONNECT_POLL_MS)) => {}
                }
                if inner.state.lock().current_session_id != session {
                    break;
                }
                if try_connect_unary(&inner.addr).await {
                    // Agent confirmed alive — resubscribe at the top. (The TS
                    // "recreate the client after 3 failures" step is a no-op
                    // here: every call already uses a fresh tonic channel.)
                    break;
                }
            }
        }
    });
}

/// Why a stream subscription returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamExit {
    /// The stream ended / errored / the 5 s first-data watchdog fired —
    /// the connection is genuinely lost and the reconnect poll must run.
    Lost,
    /// A poke arrived (session change / explicit `connectEvents`) — the TS
    /// `connectEvents()` cancels the old call and ignores its stale
    /// end/error handlers (`this.streamCall !== call` guard), so this is a
    /// SILENT resubscribe: no false notification, `connected` stays true.
    Poked,
}

/// Subscribe to the session's event stream. Loops until the stream ends,
/// errors, the 5 s first-data watchdog fires, the session changes, or a
/// poke arrives.
async fn subscribe_stream(inner: &Arc<Inner>, session: &str) -> StreamExit {
    let poke_version = inner.poke_count.load(Ordering::SeqCst);
    let connected = match future_rpc::transport::connect_channel(
        Some(&inner.addr),
        Duration::from_secs(TRY_CONNECT_TIMEOUT_SEC),
        None,
    )
    .await
    {
        Ok(connected) => connected,
        Err(_) => return StreamExit::Lost,
    };
    let mut client = FutureAgentClient::new(connected.channel);
    let request = StreamRequest {
        session_id: session.to_string(),
        ..Default::default()
    };
    let mut stream = match client.stream_events(request).await {
        Ok(resp) => resp.into_inner(),
        Err(_) => return StreamExit::Lost,
    };

    let mut connected = false;
    // TS: the connect watchdog is armed ONCE at subscribe time and cleared
    // on the FIRST data event (`if (connectWatchdog) { clearTimeout(...) }`)
    // — it is never re-armed. Long-term detection of a dead-but-open stream
    // is the heartbeat's job (tryConnect unary every 10 s). Re-arming on
    // every event would fire the watchdog on an idle stream (no events
    // flowing) and flap the connection every 5 s. The select precondition
    // below disables the arm once the first event has arrived.
    let watchdog = tokio::time::sleep(Duration::from_millis(CONNECT_WATCHDOG_MS));
    tokio::pin!(watchdog);
    loop {
        let poked = inner.poke_notify.notified();
        tokio::pin!(poked);
        poked.as_mut().enable();
        if inner.poke_count.load(Ordering::SeqCst) != poke_version {
            return StreamExit::Poked;
        }
        // Session changed — silent resubscribe (TS connectEvents semantics).
        if inner.state.lock().current_session_id != session {
            return StreamExit::Poked;
        }
        if inner.stop.load(Ordering::SeqCst) {
            return StreamExit::Lost;
        }

        tokio::select! {
            msg = stream.message() => {
                match msg {
                    Ok(Some(event)) => {
                        let raw_data: Map<String, Value> = if event.data.is_empty() {
                            Map::new()
                        } else {
                            match serde_json::from_str::<Value>(&event.data) {
                                Ok(Value::Object(map)) => map,
                                // TS: parse errors inside the data handler are
                                // swallowed and the whole event dropped.
                                _ => continue,
                            }
                        };
                        let agent_event = parse_stream_event(&event, &raw_data);

                        // Run bookkeeping (TS "data" handler).
                        if let Some(run_id) = &agent_event.run_id {
                            if agent_event.r#type == "agent_start" {
                                let mut st = inner.state.lock();
                                st.active_run_id = Some(run_id.clone());
                                st.last_run_id = Some(run_id.clone());
                                st.runs.insert(run_id.clone(), RunStatus::Running);
                            } else if agent_event.r#type == "agent_end" {
                                let mut st = inner.state.lock();
                                st.runs.insert(run_id.clone(), RunStatus::Terminal);
                                if st.active_run_id.as_deref() == Some(run_id.as_str()) {
                                    st.active_run_id = None;
                                }
                            }
                        }

                        if !connected {
                            connected = true;
                            {
                                let mut st = inner.state.lock();
                                st.connected = true;
                            }
                            let _ = inner.conn_tx.send(true);
                        }
                        if inner.event_tx.send(agent_event).is_err() {
                            return StreamExit::Lost; // app gone
                        }
                    }
                    Ok(None) => return StreamExit::Lost, // stream end
                    Err(_) => return StreamExit::Lost,   // stream error
                }
            }
            _ = &mut watchdog, if !connected => {
                // No data within 5 s of subscribing — the channel is likely
                // stuck. Cancel and let the caller run the tryConnect
                // reconnect poll. Disabled after the first event (TS clears
                // the watchdog on first data and never re-arms it).
                return StreamExit::Lost;
            }
            _ = &mut poked => {
                // Session change or explicit connect_events — silent
                // resubscribe (TS ignores the cancelled stale stream).
                return StreamExit::Poked;
            }
            _ = inner.stop_notify.notified() => {
                if inner.stop.load(Ordering::SeqCst) { return StreamExit::Lost; }
            }
        }
    }
}

/// `tryConnect()` unary (used by the reconnect poll and heartbeat).
async fn try_connect_unary(addr: &str) -> bool {
    let cmd = RpcCommand {
        id: now_id(),
        r#type: "list_models".to_string(),
        ..Default::default()
    };
    execute_unary(addr, cmd, TRY_CONNECT_TIMEOUT_SEC)
        .await
        .is_ok()
}

/// Periodic health-check: every 10 s, if connected, `tryConnect()`; on
/// failure mark disconnected (notify) and poke the manager's reconnect loop
/// (TS `startHeartbeat`).
fn spawn_heartbeat(inner: Arc<Inner>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(HEARTBEAT_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if !inner.state.lock().connected {
                        continue;
                    }
                    let alive = try_connect_unary(&inner.addr).await;
                    if !alive {
                        let was_connected = {
                            let mut st = inner.state.lock();
                            let was = st.connected;
                            st.connected = false;
                            was
                        };
                        if was_connected {
                            let _ = inner.conn_tx.send(false);
                        }
                        inner.poke_count.fetch_add(1, Ordering::SeqCst);
                        inner.poke_notify.notify_waiters();
                    }
                }
                _ = inner.stop_notify.notified() => {
                    if inner.stop.load(Ordering::SeqCst) { return; }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::provider_types::ProviderModelInput;

    fn sample_stream_event() -> StreamEvent {
        StreamEvent {
            r#type: "text_chunk".into(),
            data: r#"{"text":"Hello"}"#.into(),
            run_id: "run-1".into(),
            idx: 3,
            session_id: "s1".into(),
            epoch: 2,
            event_id: "evt".into(),
            timestamp: "2026-08-07T00:00:00Z".into(),
            session_idx: 1,
            run_sequence: 1,
            ..Default::default()
        }
    }

    #[test]
    fn parse_event_spreads_data_over_envelope() {
        let event = sample_stream_event();
        let raw: Map<String, Value> = serde_json::from_str(&event.data).expect("valid payload");
        let parsed = parse_stream_event(&event, &raw);
        assert_eq!(parsed.r#type, "text_chunk");
        assert_eq!(parsed.run_id.as_deref(), Some("run-1"));
        assert_eq!(parsed.idx, 3);
        assert_eq!(
            parsed.data.get("text").and_then(Value::as_str),
            Some("Hello")
        );
        assert_eq!(
            parsed.data.get("type").and_then(Value::as_str),
            Some("text_chunk")
        );
        assert_eq!(
            parsed.data.get("runId").and_then(Value::as_str),
            Some("run-1")
        );
        assert_eq!(parsed.data.get("epoch").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn parse_event_defaults_type_to_message() {
        let mut event = sample_stream_event();
        event.r#type = String::new();
        let raw: Map<String, Value> = Map::new();
        let parsed = parse_stream_event(&event, &raw);
        assert_eq!(parsed.r#type, "message");
        assert_eq!(
            parsed.data.get("type").and_then(Value::as_str),
            Some("message")
        );
    }

    #[test]
    fn parse_event_handles_empty_data() {
        let event = StreamEvent {
            r#type: "ping".into(),
            data: String::new(),
            ..Default::default()
        };
        let parsed = parse_stream_event(&event, &Map::new());
        assert_eq!(parsed.r#type, "ping");
        assert_eq!(
            parsed.data.get("type").and_then(Value::as_str),
            Some("ping")
        );
        assert!(parsed.data.get("text").is_none());
    }

    #[test]
    fn uuid_hex_has_no_dashes() {
        let id = uuid_hex();
        assert_eq!(id.len(), 32);
        assert!(!id.contains('-'));
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn transport_error_detection_matches_ts() {
        assert!(is_transport_error("14 UNAVAILABLE: connect ECONNREFUSED"));
        assert!(is_transport_error("transport error"));
        assert!(is_transport_error("Connect Failed"));
        assert!(is_transport_error("ECONNREFUSED"));
        assert!(!is_transport_error("2 UNKNOWN: unknown error"));
        assert!(!is_transport_error("model not found"));
    }

    #[test]
    fn run_ack_deserializes() {
        let ack: RunAck = serde_json::from_str(
            r#"{"run_id":"run-1","run_epoch":1,"accepted_state":"queued","run_sequence":2,"queue_position":1}"#,
        )
        .expect("parse");
        assert_eq!(ack.run_id, "run-1");
        assert_eq!(ack.accepted_state, "queued");
        assert_eq!(ack.queue_position, Some(1));
    }

    // ─── Stream-manager integration tests (in-process mock agent) ────────
    //
    // These exercise `spawn_stream_manager` against a real tonic server so
    // the reconnect / resubscribe / watchdog semantics are tested the way
    // they run: over an actual gRPC stream.

    use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
    use future_rpc::proto::{
        response_payload, AgentInfo, ResponsePayload, RpcCommand, RpcResponse,
        SessionStatsResponse, StatsTokens, StreamEvent, StreamRequest, SyncFutureModelsResult,
    };
    use futures_util::stream;
    use futures_util::StreamExt;
    use std::pin::Pin;
    use tokio_stream::wrappers::UnboundedReceiverStream;
    use tonic::transport::server::TcpIncoming;
    use tonic::transport::Server;

    /// Mock agent: `stream_events` emits ONE event then goes silent (idle
    /// stream, never ends). `execute_command` answers unary calls.
    #[derive(Clone)]
    struct MockAgent {
        /// Events to emit per stream subscription (first item sent, rest
        /// held back until the test pokes the sender).
        event_tx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<StreamEvent>>>>,
    }

    #[tonic::async_trait]
    impl FutureAgent for MockAgent {
        async fn execute_command(
            &self,
            request: tonic::Request<RpcCommand>,
        ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            Ok(tonic::Response::new(RpcResponse {
                id: cmd.id,
                r#type: "response".into(),
                command: cmd.r#type.clone(),
                success: true,
                data: "{}".into(),
                error: String::new(),
                error_code: String::new(),
                error_data: String::new(),
                payload: None,
            }))
        }

        type StreamEventsStream =
            Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

        async fn stream_events(
            &self,
            _request: tonic::Request<StreamRequest>,
        ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
            let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();
            *self.event_tx.lock().await = Some(tx);
            // Push a first event so the client's `connected` edge fires.
            let first = StreamEvent {
                r#type: "ping".into(),
                data: String::new(),
                ..Default::default()
            };
            let stream = UnboundedReceiverStream::new(rx);
            let stream = stream::once(async move { Ok(first) }).chain(stream.map(Ok));
            Ok(tonic::Response::new(Box::pin(stream)))
        }
    }

    /// Bind an ephemeral port, serve the mock agent, return (join handle, addr).
    async fn spawn_mock_agent() -> (
        tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        String,
    ) {
        // Serve on the listener bound here: probe-binding the port, dropping
        // it and letting tonic re-bind left a window where a concurrent mock
        // could steal it (intermittent "transport error" in the suite).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        let agent = MockAgent {
            event_tx: Arc::new(tokio::sync::Mutex::new(None)),
        };
        // Spawn the serve future directly — no async-block tail that can
        // never complete.
        let handle = tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve_with_incoming(incoming),
        );
        (handle, format!("127.0.0.1:{}", addr.port()))
    }

    /// Wait until `conn` reports connected (first stream data).
    async fn wait_connected(conn: &mut watch::Receiver<bool>) {
        wait_conn(conn, true, "never connected").await;
    }

    /// Wait until `conn` reads `want` (bounded).
    async fn wait_conn(conn: &mut watch::Receiver<bool>, want: bool, what: &str) {
        tokio::time::timeout(Duration::from_secs(10), conn.wait_for(|v| *v == want))
            .await
            .expect(what)
            .expect("conn channel closed");
    }

    /// Assert that no `false` (connection-lost) notification arrives within
    /// `dur`.
    async fn assert_no_disconnect(conn: &mut watch::Receiver<bool>, dur: Duration) {
        let deadline = tokio::time::sleep(dur);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                changed = conn.changed() => {
                    changed.expect("conn channel closed");
                    assert!(*conn.borrow());
                }
                _ = &mut deadline => break,
            }
        }
    }

    /// An idle stream (no events after the first) must NOT fire the 5 s
    /// first-data watchdog — TS arms it once and clears it on the first data
    /// event (`if (connectWatchdog) { clearTimeout(...) }`), never re-arms it.
    /// A port that re-armed the watchdog on every event flapped the
    /// connection every 5 s on an idle TUI (PTY smoke test found:
    /// "Connection to agent lost — retrying every 1s..." right after the
    /// welcome screen, before any input).
    #[tokio::test(flavor = "multi_thread")]
    async fn idle_stream_does_not_flap_after_first_data() {
        let (_server, addr) = spawn_mock_agent().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("sess-1");
        client.connect_events();

        wait_connected(&mut conn).await;
        // 6 s > 5 s watchdog: an idle stream must stay connected.
        assert_no_disconnect(&mut conn, Duration::from_millis(6_200)).await;
        assert!(client.is_connected());
        client.disconnect();
    }

    /// `setCurrentSessionId` + `connectEvents` (session change) must be a
    /// SILENT resubscribe: TS cancels the old stream and its stale end/error
    /// handlers are ignored (`this.streamCall !== call`), so no false
    /// notification and `connected` stays true. A poke-driven resubscribe
    /// that notified false produced a spurious "Connection to agent lost"
    /// on every /new during the PTY smoke test.
    #[tokio::test(flavor = "multi_thread")]
    async fn session_change_resubscribes_silently() {
        let (_server, addr) = spawn_mock_agent().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("sess-1");
        client.connect_events();

        wait_connected(&mut conn).await;

        // Session change → poke → silent resubscribe; must stay connected.
        client.set_current_session_id("sess-2");
        client.connect_events();
        assert_no_disconnect(&mut conn, Duration::from_millis(1_500)).await;
        assert!(client.is_connected());
        client.disconnect();
    }

    // ─── API surface tests (configurable mock) ───────────────────────

    use std::collections::HashMap as StdHashMap;

    /// Mock with per-command-type canned response data (swappable at
    /// runtime) and tonic Status failures, plus a command log.
    #[derive(Clone, Default)]
    struct ApiMock {
        data_by_type: Arc<std::sync::Mutex<StdHashMap<String, String>>>,
        /// Typed `payload` per command type — exercises the typed-first decode
        /// (`data` is usually left empty for these).
        payload_by_type: StdHashMap<String, ResponsePayload>,
        status_errors: StdHashMap<String, tonic::Status>,
        seen: Arc<std::sync::Mutex<Vec<String>>>,
        /// (type, session_id) of every command, for routing assertions.
        seen_sessions: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        requests: Arc<std::sync::Mutex<Vec<RpcCommand>>>,
        /// Answer success=false with this error string for these types.
        fail_with: StdHashMap<String, String>,
        /// stream_events returns a tonic error immediately.
        stream_fails: bool,
        /// The stream ends right after the first event.
        stream_ends: bool,
        /// The stream never emits anything (watchdog bait).
        stream_idle: bool,
        /// Delay before answering unary calls (slow-agent scenarios).
        unary_delay_ms: u64,
    }

    #[tonic::async_trait]
    impl FutureAgent for ApiMock {
        async fn execute_command(
            &self,
            request: tonic::Request<RpcCommand>,
        ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            self.requests.lock().unwrap().push(cmd.clone());
            self.seen.lock().unwrap().push(cmd.r#type.clone());
            self.seen_sessions
                .lock()
                .unwrap()
                .push((cmd.r#type.clone(), cmd.session_id.clone()));
            if let Some(status) = self.status_errors.get(&cmd.r#type) {
                return Err(status.clone());
            }
            if self.unary_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.unary_delay_ms)).await;
            }
            let data = self
                .data_by_type
                .lock()
                .unwrap()
                .get(&cmd.r#type)
                .cloned()
                .unwrap_or_else(|| "{}".to_string());
            let fail = self.fail_with.get(&cmd.r#type);
            Ok(tonic::Response::new(RpcResponse {
                id: cmd.id,
                r#type: "response".into(),
                command: cmd.r#type.clone(),
                success: fail.is_none(),
                data,
                error: fail.cloned().unwrap_or_default(),
                error_code: String::new(),
                error_data: String::new(),
                payload: self.payload_by_type.get(&cmd.r#type).cloned(),
            }))
        }

        type StreamEventsStream =
            Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

        async fn stream_events(
            &self,
            _request: tonic::Request<StreamRequest>,
        ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
            if self.stream_fails {
                return Err(tonic::Status::internal("stream boom"));
            }
            if self.stream_idle {
                return Ok(tonic::Response::new(Box::pin(stream::pending())));
            }
            // One event, then idle (stream stays open).
            let first = StreamEvent {
                r#type: "ping".into(),
                data: String::new(),
                ..Default::default()
            };
            let idle = stream::pending();
            let once = stream::once(async move { Ok(first) });
            if self.stream_ends {
                return Ok(tonic::Response::new(Box::pin(once)));
            }
            Ok(tonic::Response::new(Box::pin(once.chain(idle))))
        }
    }

    /// The last request of `command_type` the mock received, for wire-shape
    /// asserts.
    fn last_request(
        requests: &Arc<std::sync::Mutex<Vec<RpcCommand>>>,
        command_type: &str,
    ) -> RpcCommand {
        requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|cmd| cmd.r#type == command_type)
            .cloned()
            .unwrap_or_else(|| panic!("no {command_type} request was sent"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn provider_configuration_calls_send_the_documented_wire_shape() {
        let mock = ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                (
                    "list_providers".into(),
                    r#"{"builtin":[{"id":"future","name":"Future","baseUrl":"https://api.future.test","hasApiKey":true,"modelCount":900}],"custom":[{"id":"acme","name":"Acme","api":"openai-completions","baseUrl":"https://api.acme.test/v1","hasApiKey":false,"models":[{"id":"m1","name":"M1"}]}]}"#
                        .into(),
                ),
                (
                    "sync_future_models".into(),
                    r#"{"synced":true,"modelCount":42,"revision":7}"#.into(),
                ),
                (
                    "get_agent_info".into(),
                    r#"{"version":"1.2.3","agentInstanceId":"agent-9","skillsCount":4}"#
                        .into(),
                ),
                (
                    "export_html".into(),
                    r#"{"path":"/tmp/session.html"}"#.into(),
                ),
                ("list_tool_calls".into(), r#"{"toolCalls":[]}"#.into()),
                (
                    "get_tool_output".into(),
                    r#"{"output":"hello"}"#.into(),
                ),
                (
                    "search_session_history".into(),
                    r#"{"hits":[]}"#.into(),
                ),
            ]))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("sess-1");

        // ── Providers / auth ───────────────────────────────────────────
        let providers = client.list_providers().await.unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].id, "future");
        assert!(providers[0].builtin);
        assert_eq!(providers[0].model_count, 900);
        assert_eq!(providers[1].id, "acme");
        assert_eq!(providers[1].models.len(), 1);

        let input = ProviderInput {
            id: "acme".into(),
            name: "Acme".into(),
            api_type: "openai-completions".into(),
            base_url: "https://api.acme.test/v1".into(),
            models: vec![ProviderModelInput::new("m1", "M1")],
            api_key: Some("sk-test".into()),
            clear_api_key: false,
            create_only: true,
        };
        client.upsert_provider(&input).await.unwrap();
        let sent = last_request(&requests, "upsert_provider");
        let spec = sent.provider_config.clone().expect("provider_config");
        assert_eq!(spec.id, "acme");
        assert_eq!(spec.name, "Acme");
        assert_eq!(spec.api, "openai-completions");
        assert_eq!(spec.base_url, "https://api.acme.test/v1");
        assert_eq!(spec.api_key, "sk-test");
        assert!(spec.replace_models);
        assert!(spec.create_only);
        assert_eq!(spec.models.len(), 1);
        assert_eq!(spec.models[0].id, "m1");
        assert_eq!(spec.models[0].modalities, vec!["text".to_string()]);
        assert_eq!(spec.models[0].reasoning, Some(true));

        client.delete_provider("acme").await.unwrap();
        let spec = last_request(&requests, "delete_provider")
            .provider_config
            .expect("provider_config");
        assert_eq!(spec.id, "acme");
        // delete_provider only reads the id — nothing else may travel with it.
        assert_eq!(
            spec,
            future_rpc::proto::ProviderUpsert {
                id: "acme".into(),
                ..Default::default()
            }
        );

        client.set_auth_key("future", Some("sk-x")).await.unwrap();
        let update = last_request(&requests, "set_auth")
            .auth_update
            .expect("auth_update");
        assert_eq!(update.provider, "future");
        assert_eq!(update.key, "sk-x");
        assert!(!update.clear_key);
        // `None` and whitespace-only both clear the stored key.
        for key in [None, Some("   ")] {
            client.set_auth_key("future", key).await.unwrap();
            let update = last_request(&requests, "set_auth")
                .auth_update
                .expect("auth_update");
            assert_eq!(update.key, "");
            assert!(update.clear_key);
        }

        client.reload_auth().await.unwrap();
        assert!(!last_request(&requests, "reload_auth").id.is_empty());
        let synced = client.sync_future_models().await.unwrap();
        assert_eq!(synced.get("modelCount").and_then(Value::as_i64), Some(42));
        client.set_default_model("future/gpt-5").await.unwrap();
        assert_eq!(
            last_request(&requests, "set_default_model").model_id,
            "future/gpt-5"
        );
        let info = client.get_agent_info().await.unwrap();
        assert_eq!(info.get("version").and_then(Value::as_str), Some("1.2.3"));

        // ── Session inspection ─────────────────────────────────────────
        // The run-scoped reads address the run the client tracks (here: a
        // second mock whose `prompt` answers with a real run ack).
        let ack_data = "{\"run_id\":\"r1\",\"run_epoch\":1,\"accepted_state\":\"running\"}";
        let mock2 = ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                ("prompt".into(), ack_data.to_string()),
                ("list_tool_calls".into(), r#"{"toolCalls":[]}"#.into()),
                ("get_tool_output".into(), r#"{"output":"hello"}"#.into()),
            ]))),
            ..Default::default()
        };
        let requests2 = mock2.requests.clone();
        let addr2 = spawn_api_mock(mock2).await;
        let (client2, _events2, _conn2) = GrpcClient::new(&addr2);
        client2.set_current_session_id("sess-1");
        client2
            .prompt("go", "enqueue_if_busy", Vec::new())
            .await
            .unwrap();
        client2.list_tool_calls().await.unwrap();
        let cmd = last_request(&requests2, "list_tool_calls");
        assert_eq!(cmd.run_id, "r1");
        assert_eq!(cmd.session_id, "sess-1");
        assert_eq!(
            client2.get_tool_output("tool-1").await.unwrap()["output"],
            "hello"
        );
        let cmd = last_request(&requests2, "get_tool_output");
        assert_eq!(cmd.run_id, "r1");
        assert_eq!(cmd.tool_call_id.as_deref(), Some("tool-1"));
        client2.disconnect();

        // ── Session settings ───────────────────────────────────────────
        client.search_session_history("needle", 5).await.unwrap();
        let cmd = last_request(&requests, "search_session_history");
        assert_eq!(cmd.message, "needle");
        assert_eq!(cmd.limit, Some(5));
        assert_eq!(cmd.session_id, "sess-1");
        client.set_auto_compaction(true).await.unwrap();
        assert!(last_request(&requests, "set_auto_compaction").enabled);
        client.set_auto_retry(false).await.unwrap();
        assert!(!last_request(&requests, "set_auto_retry").enabled);
        client
            .set_tools(&["read".to_string(), "write".to_string()])
            .await
            .unwrap();
        assert_eq!(
            last_request(&requests, "set_tools").tools,
            vec!["read".to_string(), "write".to_string()]
        );
        client.disable_tools().await.unwrap();
        assert_eq!(
            last_request(&requests, "disable_tools").session_id,
            "sess-1"
        );
        let export = client.export_html().await.unwrap();
        assert_eq!(
            export.get("path").and_then(Value::as_str),
            Some("/tmp/session.html")
        );
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn provider_configuration_errors_and_empty_views_surface() {
        let mock = ApiMock {
            // list_providers/custom arrays missing entirely (an agent with no
            // configured providers) — the list is empty, not an error. The
            // prompt answers a live run so the run-scoped reads reach the
            // agent (and its failures) instead of the client-side guard.
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                ("list_providers".into(), String::new()),
                (
                    "prompt".into(),
                    "{\"run_id\":\"r1\",\"run_epoch\":1,\"accepted_state\":\"running\"}".into(),
                ),
            ]))),
            fail_with: StdHashMap::from([
                (
                    "upsert_provider".into(),
                    "provider_config.id is empty".into(),
                ),
                (
                    "delete_provider".into(),
                    "reserved for a built-in provider".into(),
                ),
                ("set_auth".into(), "auth_update.provider is empty".into()),
                ("reload_auth".into(), "reload failed".into()),
                ("sync_future_models".into(), "network down".into()),
                (
                    "set_default_model".into(),
                    "model is not in the catalog".into(),
                ),
                ("get_session_stats".into(), "no session".into()),
                ("export_html".into(), "failed to write file".into()),
                ("set_tools".into(), "unknown tool".into()),
                ("search_session_history".into(), "session not found".into()),
                ("set_auto_compaction".into(), "busy".into()),
                ("set_auto_retry".into(), "busy".into()),
                ("disable_tools".into(), "busy".into()),
                ("append_system_prompt".into(), "prompt too long".into()),
            ]),
            status_errors: StdHashMap::from([
                (
                    "get_agent_info".into(),
                    tonic::Status::unavailable("agent gone"),
                ),
                (
                    "get_tool_output".into(),
                    tonic::Status::internal("tool output boom"),
                ),
            ]),
            ..Default::default()
        };
        let seen = mock.seen.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("sess-1");

        // A payload the parser cannot read yields an empty list, never an error.
        assert!(client.list_providers().await.unwrap().is_empty());

        let input = crate::rpc::provider_types::ProviderInput {
            id: "acme".into(),
            name: "Acme".into(),
            api_type: "openai-completions".into(),
            base_url: "https://api.acme.test/v1".into(),
            ..Default::default()
        };
        assert_eq!(
            client.upsert_provider(&input).await.unwrap_err(),
            "provider_config.id is empty"
        );
        assert_eq!(
            client.delete_provider("future").await.unwrap_err(),
            "reserved for a built-in provider"
        );
        assert_eq!(
            client.set_auth_key("", None).await.unwrap_err(),
            "auth_update.provider is empty"
        );
        assert_eq!(client.reload_auth().await.unwrap_err(), "reload failed");
        assert_eq!(
            client.sync_future_models().await.unwrap_err(),
            "network down"
        );
        assert_eq!(
            client.set_default_model("nope").await.unwrap_err(),
            "model is not in the catalog"
        );
        assert!(client
            .get_agent_info()
            .await
            .unwrap_err()
            .contains("agent gone"));
        assert_eq!(client.get_session_stats().await.unwrap_err(), "no session");
        // Run-scoped read against a live (mock) run: the agent answers, so the
        // call succeeds (an empty payload still decodes to an object).
        client
            .prompt("go", "enqueue_if_busy", Vec::new())
            .await
            .unwrap();
        assert!(client.list_tool_calls().await.is_ok());
        assert_eq!(
            client.export_html().await.unwrap_err(),
            "failed to write file"
        );
        assert_eq!(
            client.set_tools(&["nope".to_string()]).await.unwrap_err(),
            "unknown tool"
        );
        assert_eq!(
            client.search_session_history("x", 1).await.unwrap_err(),
            "session not found"
        );
        assert!(client
            .get_tool_output("tool-1")
            .await
            .unwrap_err()
            .contains("tool output boom"));
        // The remaining wrappers take no interesting input — an error response
        // still has to surface instead of being swallowed.
        for result in [
            client.set_auto_compaction(true).await,
            client.set_auto_retry(true).await,
            client.disable_tools().await,
        ] {
            assert!(result.is_err(), "a failed response must surface");
        }
        assert!(!seen.lock().unwrap().is_empty());
        client.disconnect();
    }

    /// Client-side validation runs before the wire: an invalid provider never
    /// reaches the agent (and the form still learns why).
    #[tokio::test(flavor = "multi_thread")]
    async fn upsert_provider_validates_before_the_wire() {
        let mock = ApiMock::default();
        let seen = mock.seen.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        let invalid = ProviderInput {
            id: "Bad Id".into(),
            api_type: "openai-completions".into(),
            base_url: "https://api.acme.test/v1".into(),
            ..Default::default()
        };
        assert_eq!(
            client.upsert_provider(&invalid).await.unwrap_err(),
            "provider id must use lowercase letters, digits, '-' or '_'"
        );
        assert!(!seen.lock().unwrap().iter().any(|t| t == "upsert_provider"));
        client.disconnect();
    }

    /// A run-scoped read must keep working after the run ends (`active_run_id`
    /// is cleared on `agent_end`) and only fail when there was never a run.
    #[tokio::test(flavor = "multi_thread")]
    async fn tool_reads_use_the_active_then_the_last_run() {
        let mock = ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                (
                    "prompt".into(),
                    "{\"run_id\":\"r1\",\"run_epoch\":1,\"accepted_state\":\"running\"}".into(),
                ),
                ("get_state".into(), "{\"sessionId\":\"s1\"}".into()),
                ("list_tool_calls".into(), "{\"toolCalls\":[]}".into()),
            ]))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");

        // Never ran anything in this session.
        assert_eq!(
            client.list_tool_calls().await.unwrap_err(),
            "no run to list tool calls for"
        );
        assert_eq!(
            client.get_tool_output("tool-1").await.unwrap_err(),
            "no run to read tool output for"
        );

        client
            .prompt("go", "enqueue_if_busy", Vec::new())
            .await
            .unwrap();
        client.list_tool_calls().await.unwrap();
        assert_eq!(last_request(&requests, "list_tool_calls").run_id, "r1");

        // The run ends: get_state reports no active run, the last one is kept.
        client.get_state().await.unwrap();
        client.list_tool_calls().await.unwrap();
        assert_eq!(last_request(&requests, "list_tool_calls").run_id, "r1");

        // A different session has no run of its own.
        client.set_current_session_id("s2");
        assert!(client.list_tool_calls().await.is_err());
        client.disconnect();
    }

    /// Typed `payload` responses decode ahead of the JSON `data` fallback
    /// (get_agent_info / get_session_stats / sync_future_models are typed).
    #[tokio::test(flavor = "multi_thread")]
    async fn typed_payloads_decode_and_json_data_still_falls_back() {
        let mock = ApiMock {
            payload_by_type: StdHashMap::from([
                (
                    "get_agent_info".into(),
                    ResponsePayload {
                        kind: Some(response_payload::Kind::GetAgentInfo(AgentInfo {
                            version: "9.9.9".into(),
                            agent_instance_id: "agent-typed".into(),
                            skills_count: 3,
                        })),
                    },
                ),
                (
                    "sync_future_models".into(),
                    ResponsePayload {
                        kind: Some(response_payload::Kind::SyncFutureModels(
                            SyncFutureModelsResult {
                                synced: true,
                                model_count: 1_234,
                                revision: 5,
                            },
                        )),
                    },
                ),
                (
                    "get_session_stats".into(),
                    ResponsePayload {
                        kind: Some(response_payload::Kind::GetSessionStats(
                            SessionStatsResponse {
                                session_file: "/tmp/s.jsonl".into(),
                                session_id: "s1".into(),
                                user_messages: 2,
                                assistant_messages: 3,
                                tool_calls: 4,
                                tool_results: 4,
                                total_messages: 9,
                                tokens: Some(StatsTokens {
                                    input: 100,
                                    output: 50,
                                    cache_read: 10,
                                    total: 160,
                                }),
                                cost: 1.5,
                            },
                        )),
                    },
                ),
            ]),
            // Stale JSON `data` for the same commands: the typed payload wins.
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                (
                    "get_agent_info".into(),
                    "{\"version\":\"0.0.0\",\"agentInstanceId\":\"stale\"}".into(),
                ),
                (
                    "sync_future_models".into(),
                    "{\"synced\":false,\"modelCount\":1}".into(),
                ),
            ]))),
            ..Default::default()
        };
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");

        let info = client.get_agent_info().await.unwrap();
        assert_eq!(info["version"], "9.9.9");
        assert_eq!(info["agentInstanceId"], "agent-typed");
        assert_eq!(info["skillsCount"], 3);

        let synced = client.sync_future_models().await.unwrap();
        assert_eq!(synced["synced"], true);
        assert_eq!(synced["modelCount"], 1_234);
        assert_eq!(synced["revision"], 5);

        let stats = client.get_session_stats().await.unwrap();
        assert_eq!(stats["sessionId"], "s1");
        assert_eq!(stats["toolCalls"], 4);
        assert_eq!(stats["tokens"]["total"], 160);
        assert_eq!(stats["cost"], 1.5);
        client.disconnect();
    }

    async fn spawn_api_mock(mock: ApiMock) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        // Spawn the serve future directly (no async block → no never-taken
        // completion tail).
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(mock))
                .serve_with_incoming(incoming),
        );
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn no_context_files_follows_session_changes_and_precedes_requests() {
        let mock = ApiMock::default();
        let requests = mock.requests.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        let client = client.with_no_context_files(true);
        for session_id in ["first-session", "new-session"] {
            client.set_current_session_id(session_id);
            wait_connected(&mut conn).await;
            for command in ["get_state", "reload_config", "prompt"] {
                client.call(command, RpcCommand::default()).await.unwrap();
            }
        }
        let requests = requests.lock().unwrap();
        let relevant: Vec<_> = requests
            .iter()
            .filter(|cmd| cmd.r#type != "list_models")
            .collect();
        assert_eq!(relevant.len(), 12);
        for (pair, session_id) in relevant.chunks_exact(2).zip([
            "first-session",
            "first-session",
            "first-session",
            "new-session",
            "new-session",
            "new-session",
        ]) {
            assert_eq!(pair[0].r#type, "set_context_files");
            assert!(!pair[0].enabled);
            assert_eq!(pair[0].session_id, session_id);
            assert_eq!(pair[1].session_id, session_id);
        }
        client.disconnect();
    }

    #[tokio::test]
    async fn no_context_files_rejection_prevents_prompt() {
        let mock = ApiMock {
            fail_with: StdHashMap::from([("set_context_files".into(), "unsupported".into())]),
            ..Default::default()
        };
        let seen = mock.seen.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        let client = client.with_no_context_files(true);
        client.set_current_session_id("s1");
        wait_connected(&mut conn).await;
        assert_eq!(
            client
                .prompt("hi", "enqueue_if_busy", Vec::new())
                .await
                .unwrap_err(),
            "unsupported"
        );
        assert!(!seen.lock().unwrap().iter().any(|cmd| cmd == "prompt"));
        client.disconnect();
    }

    // ─── Sandbox / skills / lifecycle wrappers ─────────────────────────

    /// Each wrapper added with the TUI-parity round puts its argument on the
    /// exact field the agent reads (`sandbox_policy.tier`, `mode`, `run_id`,
    /// `command` + `shell_timeout_ms`, …) and hands the response back verbatim
    /// — including the sandbox downgrade, which the wrapper deliberately does
    /// not flatten into `()`.
    #[tokio::test(flavor = "multi_thread")]
    async fn sandbox_skills_and_lifecycle_wrappers_reach_the_wire() {
        let mock = ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                (
                    "probe_sandbox".into(),
                    r#"{"available":true,"code":"available","backend":"macos_seatbelt"}"#.into(),
                ),
                (
                    "probe_windows_sandbox".into(),
                    r#"{"available":false,"code":"platform_unsupported"}"#.into(),
                ),
                (
                    "set_sandbox_policy".into(),
                    r#"{"tier":"manual","requestedTier":"sandbox","sandboxAvailable":false,"sandboxCode":"binary_missing"}"#
                        .into(),
                ),
                ("get_commands".into(), r#"{"commands":[]}"#.into()),
                (
                    "refresh_skills".into(),
                    r#"{"refreshed":true,"skills":[],"skills_count":0}"#.into(),
                ),
                ("delete_session".into(), r#"{"sessionId":"s2"}"#.into()),
                ("generate_session_title".into(), r#"{"title":"hi"}"#.into()),
                ("get_runtime_metrics".into(), r#"{"activeRunGauge":0}"#.into()),
                (
                    "get_run_snapshot".into(),
                    r#"{"runSnapshot":true,"watermark":3}"#.into(),
                ),
                ("shell".into(), r#"{"output":"hi\n","exitCode":0}"#.into()),
            ]))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);

        let probe = client.probe_sandbox().await.unwrap();
        assert_eq!(probe["backend"], "macos_seatbelt");
        assert_eq!(
            client.probe_windows_sandbox().await.unwrap()["code"],
            "platform_unsupported"
        );
        // The downgrade survives the wrapper: requested sandbox, got manual.
        let policy = client.set_sandbox_policy("sandbox").await.unwrap();
        assert_eq!(policy["tier"], "manual");
        assert_eq!(policy["requestedTier"], "sandbox");
        assert_eq!(
            client.get_commands().await.unwrap()["commands"],
            serde_json::json!([])
        );
        assert!(client.refresh_skills().await.unwrap()["refreshed"]
            .as_bool()
            .unwrap());
        assert_eq!(
            client.delete_session("s2").await.unwrap()["sessionId"],
            "s2"
        );
        assert_eq!(
            client.generate_session_title("zh").await.unwrap()["title"],
            "hi"
        );
        assert_eq!(
            client.get_runtime_metrics().await.unwrap()["activeRunGauge"],
            0
        );
        assert_eq!(
            client.get_run_snapshot("run-9").await.unwrap()["watermark"],
            3
        );
        client.set_context_files(false).await.unwrap();
        assert_eq!(client.shell("ls -la", 0).await.unwrap()["exitCode"], 0);
        assert_eq!(client.shell("ls", 5_000).await.unwrap()["output"], "hi\n");

        let sent = requests.lock().unwrap().clone();
        let last = |command: &str| {
            sent.iter()
                .rfind(|cmd| cmd.r#type == command)
                .cloned()
                .unwrap_or_else(|| panic!("{command} never reached the agent"))
        };
        assert_eq!(
            last("set_sandbox_policy")
                .sandbox_policy
                .as_ref()
                .unwrap()
                .tier,
            "sandbox"
        );
        assert_eq!(last("delete_session").session_id, "s2");
        assert_eq!(last("generate_session_title").mode, "zh");
        assert_eq!(last("get_run_snapshot").run_id, "run-9");
        assert!(!last("set_context_files").enabled);
        assert_eq!(last("shell").command, "ls");
        assert_eq!(last("shell").shell_timeout_ms, 5_000);
        // The probes and the skills catalogue carry no arguments at all.
        assert_eq!(last("probe_sandbox").session_id, "");
        assert_eq!(last("refresh_skills").session_id, "");
        client.disconnect();
    }

    /// A failed write is an `Err`, never a silent `Ok(())`; and the run-scoped
    /// snapshot names its own client-side reason when the session has no run.
    #[tokio::test(flavor = "multi_thread")]
    async fn lifecycle_wrapper_failures_surface_and_snapshot_needs_a_run() {
        let mock = ApiMock {
            fail_with: StdHashMap::from([("set_context_files".into(), "nope".into())]),
            ..Default::default()
        };
        let addr = spawn_api_mock(mock).await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        assert_eq!(client.set_context_files(true).await.unwrap_err(), "nope");
        assert_eq!(
            client.snapshot_run_id().unwrap_err(),
            "no run to snapshot yet"
        );
        client.disconnect();
    }

    #[test]
    fn grpc_addr_env_override() {
        let _guard = crate::test_env::lock();
        fn restore(key: &str, old: Option<std::ffi::OsString>) {
            match old {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        let old = std::env::var_os("FUTURE_AGENT_GRPC_ADDR");
        std::env::remove_var("FUTURE_AGENT_GRPC_ADDR");
        assert_eq!(grpc_addr(), "auto");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "example:1234");
        assert_eq!(grpc_addr(), "example:1234");
        // Both restore arms.
        restore("FUTURE_AGENT_GRPC_ADDR", Some("ambient".into()));
        assert_eq!(grpc_addr(), "ambient");
        restore("FUTURE_AGENT_GRPC_ADDR", None);
        assert_eq!(grpc_addr(), "auto");
        restore("FUTURE_AGENT_GRPC_ADDR", old);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_management_calls() {
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                ("new_session".into(), "{\"sessionId\":\"s-new\"}".into()),
                ("switch_session".into(), "{\"cancelled\":false}".into()),
                ("fork".into(), "{\"sessionId\":\"s-fork\"}".into()),
                ("clone".into(), "{\"sessionId\":\"s-clone\"}".into()),
                ("get_fork_messages".into(), "{\"messages\":[]}".into()),
                (
                    "list_sessions".into(),
                    "{\"sessions\":[{\"id\":\"s1\",\"cwd\":\"/tmp\",\"updatedAt\":\"2026-01-01\",\"model\":\"m\"}, {\"bad\":true}]}"
                        .into(),
                ),
            ]))),
            seen: Arc::new(std::sync::Mutex::new(Vec::new())),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);

        // new_session with a sessionId updates the current session.
        let v = client
            .new_session(None, Some("m"), Some("high"))
            .await
            .unwrap();
        assert_eq!(v.get("sessionId").and_then(Value::as_str), Some("s-new"));
        assert_eq!(client.get_current_session_id(), "s-new");

        // switch (not cancelled) → current session changes.
        let v = client.switch_session("s-2").await.unwrap();
        assert_eq!(v.get("cancelled").and_then(Value::as_bool), Some(false));
        assert_eq!(client.get_current_session_id(), "s-2");

        // fork + clone pick up their returned session ids.
        client.fork("entry-1").await.unwrap();
        assert_eq!(client.get_current_session_id(), "s-fork");
        client.clone_session().await.unwrap();
        assert_eq!(client.get_current_session_id(), "s-clone");

        // Plain calls.
        client.get_fork_messages().await.unwrap();
        client.set_session_name("name").await.unwrap();
        let sessions = client.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1); // the malformed entry is dropped
        assert_eq!(sessions[0].id, "s1");
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_session_does_not_inherit_current_session_id() {
        // Regression: `call` used to backfill the current session id onto
        // every command with an empty one — including `new_session`, which
        // the agent then read as "create/restore THAT id", handing back the
        // same session with its full history (so /new looked like a no-op).
        let seen_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                ("new_session".into(), "{\"sessionId\":\"s-new\"}".into()),
                ("get_state".into(), "{\"sessionId\":\"s-new\"}".into()),
            ]))),
            seen_sessions: seen_sessions.clone(),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s-old");

        client.new_session(None, None, None).await.unwrap();
        assert_eq!(client.get_current_session_id(), "s-new");
        // Other commands still inherit the current session id (routing).
        client.get_state().await.unwrap();

        let seen = seen_sessions.lock().unwrap().clone();
        assert!(
            seen.iter()
                .any(|(t, sid)| t == "new_session" && sid.is_empty()),
            "new_session must be sent session-less, saw {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|(t, sid)| t == "get_state" && sid == "s-new"),
            "other commands keep routing by session id, saw {seen:?}"
        );
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn switch_session_cancelled_keeps_current() {
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([(
                "switch_session".into(),
                "{\"cancelled\":true}".into(),
            )]))),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("keep-me");
        let before = client.get_current_session_id();
        client.switch_session("other").await.unwrap();
        assert_eq!(client.get_current_session_id(), before);
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn core_rpc_calls_and_run_bookkeeping() {
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                (
                    "prompt".into(),
                    "{\"run_id\":\"r1\",\"run_epoch\":1,\"accepted_state\":\"running\"}".into(),
                ),
                (
                    "get_state".into(),
                    "{\"sessionId\":\"s1\",\"activeRun\":{\"runId\":\"r1\",\"epoch\":1,\"state\":\"running\",\"lastEventIdx\":0},\"queuedRuns\":[{\"runId\":\"q1\",\"runSequence\":1,\"clientRequestId\":\"req-1\",\"queuePosition\":1,\"acceptedAt\":\"2026-01-01\",\"displayText\":\"hello\"}],\"agentInstanceId\":\"agent-1\"}"
                        .into(),
                ),
                ("get_messages".into(), "{\"messages\":[]}".into()),
                ("cycle_model".into(), "{\"model\":\"m2\"}".into()),
                (
                    "list_models".into(),
                    "{\"models\":[{\"id\":\"gpt-4o\",\"label\":\"GPT-4o\",\"provider\":\"openai\"}, {\"bad\":true}]}".into(),
                ),
                ("cycle_thinking_level".into(), "{\"level\":\"high\"}".into()),
                (
                    "compact".into(),
                    "{\"accepted\":true,\"operationId\":\"cmp-1\"}".into(),
                ),
                ("reload_config".into(), "{\"reloaded\":true}".into()),
            ]))),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");

        // prompt (running) → run tracked as active.
        let ack = client.prompt("hello", "queue", Vec::new()).await.unwrap();
        assert_eq!(ack.run_id, "r1");
        assert!(client.has_running_run());

        // get_state reconciles runs from the server view.
        let state = client.get_state().await.unwrap();
        assert_eq!(state.session_id, "s1");
        assert!(client.has_running_run());

        client.abort().await.unwrap();
        client.cancel_queued_run("q1").await.unwrap();
        client.get_messages().await.unwrap();
        client.set_model("m2").await.unwrap();
        client.cycle_model().await.unwrap();
        let models = client.list_models().await.unwrap();
        assert_eq!(models.len(), 1);
        client.set_thinking_level("high").await.unwrap();
        client.cycle_thinking_level().await.unwrap();
        client.compact(Some("focus on x")).await.unwrap();
        client.set_cwd("/tmp").await.unwrap();
        client
            .approval_decision("tool-1", true, "looks safe")
            .await
            .unwrap();
        client
            .approval_decision("tool-2", false, "too risky")
            .await
            .unwrap();
        client.set_permission_level("auto").await.unwrap();
        client.reload_config().await.unwrap();
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_queued_state_and_lost_run_detection() {
        let data = Arc::new(std::sync::Mutex::new(StdHashMap::from([
            (
                "prompt".to_string(),
                "{\"run_id\":\"q1\",\"run_epoch\":1,\"accepted_state\":\"queued\",\"queue_position\":1}".to_string(),
            ),
            (
                "get_state".to_string(),
                "{\"sessionId\":\"s1\",\"agentInstanceId\":\"agent-1\",\"queuedRuns\":[{\"runId\":\"q1\",\"runSequence\":1,\"clientRequestId\":\"req-1\",\"queuePosition\":1,\"acceptedAt\":\"2026-01-01\",\"displayText\":\"hi\"}]}".to_string(),
            ),
        ])));
        let addr = spawn_api_mock(ApiMock {
            data_by_type: data.clone(),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");

        let ack = client.prompt("later", "queue", Vec::new()).await.unwrap();
        assert_eq!(ack.accepted_state, "queued");
        assert!(!client.has_running_run());

        // First get_state registers agent-1 and the queued run.
        client.get_state().await.unwrap();
        // The agent restarts (instance id changes) → the queued run the
        // client still tracks is reported lost.
        data.lock().unwrap().insert(
            "get_state".to_string(),
            "{\"sessionId\":\"s1\",\"agentInstanceId\":\"agent-2\",\"queuedRuns\":[]}".to_string(),
        );
        // Requeue the run locally so there is something to lose.
        client.prompt("again", "queue", Vec::new()).await.unwrap();
        client.get_state().await.unwrap();
        assert_eq!(client.take_lost_queued_run_ids(), vec!["q1".to_string()]);
        // Second take drains.
        assert!(client.take_lost_queued_run_ids().is_empty());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_unary_edge_cases() {
        // Status error with a message.
        let addr = spawn_api_mock(ApiMock {
            status_errors: StdHashMap::from([(
                "get_state".into(),
                tonic::Status::unavailable("connection refused"),
            )]),
            ..Default::default()
        })
        .await;
        let err = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap_err();
        assert!(err.contains("connection refused"));

        // Empty data → Null; non-JSON data now also decodes to Null
        // (typed-first decode, no string passthrough).
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([
                ("get_state".into(), String::new()),
                ("get_messages".into(), "raw text".into()),
            ]))),
            ..Default::default()
        })
        .await;
        let v = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap();
        assert!(v.is_null());
        let v = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_messages".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap();
        assert!(v.is_null());
    }

    /// One live `stream_events` subscription.
    type EventSubscriber = mpsc::UnboundedSender<Result<StreamEvent, tonic::Status>>;
    /// Every live subscription, in creation order.
    type EventSubscribers = Arc<Mutex<Vec<EventSubscriber>>>;

    /// Test-owned event source behind [`EventfulMock`].
    ///
    /// Every `stream_events` call is served a live stream of its own, the way
    /// the real agent behaves: a session change or `connectEvents` makes the
    /// client resubscribe, and the new subscription has to keep receiving. (The
    /// previous single-shot channel handed the *first* subscription the real
    /// receiver and every resubscribe an empty `pending()` stream, so a poke
    /// that landed while the client was already attached killed the stream and
    /// the test's next send panicked with `SendError`.)
    ///
    /// A poke can also make the manager abandon a subscription it has just
    /// created — it returns `Poked` at the top of its loop *before* reading a
    /// message — so a test must prove delivery (see `attach`) rather than wait
    /// for a subscription to exist.
    #[derive(Clone, Default)]
    struct EventSource {
        subscribers: EventSubscribers,
        /// `stream_events` calls served so far (monotonic).
        subscribes: Arc<std::sync::atomic::AtomicUsize>,
        /// Woken on every subscribe.
        attached: Arc<Notify>,
    }

    impl EventSource {
        /// Serve one `stream_events` call: a live stream of its own.
        fn stream(&self) -> UnboundedReceiverStream<Result<StreamEvent, tonic::Status>> {
            let (tx, rx) = mpsc::unbounded_channel();
            self.subscribers.lock().push(tx);
            self.subscribes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.attached.notify_waiters();
            UnboundedReceiverStream::new(rx)
        }

        /// Deliver one event to every live subscription, pruning the streams
        /// the client has dropped. Errors when nothing was subscribed: the
        /// event would be dropped and the test would hang on a missing
        /// assertion, so fail at the send instead.
        fn send(&self, event: Result<StreamEvent, tonic::Status>) -> Result<usize, &'static str> {
            let mut delivered = 0usize;
            self.subscribers.lock().retain(|subscriber| {
                let ok = subscriber.send(event.clone()).is_ok();
                delivered += usize::from(ok);
                ok
            });
            if delivered == 0 {
                return Err("no live stream_events subscription to deliver to");
            }
            Ok(delivered)
        }

        /// Wait until `count` `stream_events` calls have been served — i.e. the
        /// client's stream manager (or its resubscription) is attached.
        async fn wait_for_subscribes(&self, count: usize) {
            loop {
                // Register before re-checking: `notify_waiters` only wakes
                // waiters that are already registered.
                let attached = self.attached.notified();
                tokio::pin!(attached);
                attached.as_mut().enable();
                if self.subscribe_count() >= count {
                    return;
                }
                attached.await;
            }
        }

        /// `stream_events` calls served so far.
        fn subscribe_count(&self) -> usize {
            self.subscribes.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[derive(Clone)]
    struct EventfulMock {
        source: EventSource,
    }

    #[tonic::async_trait]
    impl FutureAgent for EventfulMock {
        async fn execute_command(
            &self,
            request: tonic::Request<RpcCommand>,
        ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            Ok(tonic::Response::new(RpcResponse {
                id: cmd.id,
                r#type: "response".into(),
                command: cmd.r#type.clone(),
                success: true,
                data: "{}".into(),
                error: String::new(),
                error_code: String::new(),
                error_data: String::new(),
                payload: None,
            }))
        }
        type StreamEventsStream =
            Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;
        async fn stream_events(
            &self,
            _request: tonic::Request<StreamRequest>,
        ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
            Ok(tonic::Response::new(Box::pin(self.source.stream())))
        }
    }

    async fn spawn_eventful_mock() -> (EventSource, String) {
        let source = EventSource::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        let mock = EventfulMock {
            source: source.clone(),
        };
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(mock))
                .serve_with_incoming(incoming),
        );
        (source, format!("127.0.0.1:{}", addr.port()))
    }

    #[allow(clippy::result_large_err)] // mock helper mirrors the real stream error type
    fn stream_event(t: &str, data: &str, run_id: &str) -> Result<StreamEvent, tonic::Status> {
        Ok(StreamEvent {
            r#type: t.into(),
            data: data.into(),
            run_id: run_id.into(),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn session_poke_between_empty_read_and_wait_is_not_lost() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, _conn) = GrpcClient::new(&addr);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        *client.inner.before_empty_wait.lock() = Some(barrier.clone());
        barrier.wait().await;
        client.set_current_session_id("s1");
        barrier.wait().await;
        // The manager subscribes only after the barrier releases it, so wait
        // for that subscription rather than hoping a send is buffered.
        tx.wait_for_subscribes(1).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .is_some());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_stream_event_bookkeeping() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, _conn) = GrpcClient::new(&addr);
        // A unary call exercises the mock's execute_command.
        assert!(client.try_connect().await);
        attach(&client, &tx, &mut events).await;

        // Malformed data is dropped without disturbing the stream.
        tx.send(stream_event("text_chunk", "not json", "")).unwrap();
        // A run-scoped event that is neither start nor end skips the
        // bookkeeping arms.
        tx.send(stream_event("text_chunk", "{\"text\":\"x\"}", "r0"))
            .unwrap();
        // agent_start marks the run active…
        tx.send(stream_event("agent_start", "{}", "r1")).unwrap();
        let mut seen = recv_until(&mut events, "agent_start").await;
        assert!(client.has_running_run());
        // agent_end for a DIFFERENT run keeps the active run. The ping that
        // follows is delivered in order, so once it arrives the end was
        // handled (no sleep needed for the negative assertion).
        tx.send(stream_event("agent_end", "{}", "r9")).unwrap();
        tx.send(stream_event("ping", "{}", "")).unwrap();
        seen.extend(recv_until(&mut events, "ping").await);
        assert!(client.has_running_run());
        // …and agent_end for the active run clears it.
        tx.send(stream_event("agent_end", "{}", "r1")).unwrap();
        tx.send(stream_event("ping", "{}", "")).unwrap();
        seen.extend(recv_until(&mut events, "ping").await);
        assert!(!client.has_running_run());
        // The events also flowed to the app channel.
        assert!(seen.contains(&"agent_start".to_string()), "saw {seen:?}");
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_loop_top_session_check() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, mut conn) = GrpcClient::new(&addr);
        attach(&client, &tx, &mut events).await;
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // Session changed WITHOUT a poke (white-box): the next event drives
        // the loop iteration whose top check silently resubscribes.
        let before = tx.subscribe_count();
        client.inner.state.lock().current_session_id = "s2".into();
        tx.send(stream_event("text_chunk", "{}", "")).unwrap();
        tx.wait_for_subscribes(before + 1).await;
        // The resubscribed stream is live too: a later event still arrives.
        tx.send(stream_event("ping", "{}", "")).unwrap();
        recv_until(&mut events, "ping").await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_event_channel_closed_is_lost() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, mut conn) = GrpcClient::new(&addr);
        attach(&client, &tx, &mut events).await;
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // App event channel dropped → the next event send fails → Lost.
        drop(events);
        tx.send(stream_event("ping", "{}", "")).unwrap();
        wait_conn(&mut conn, false, "disconnect after the app channel closes").await;
        assert!(!client.is_connected());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_loop_top_stop_check() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, mut conn) = GrpcClient::new(&addr);
        attach(&client, &tx, &mut events).await;
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // Stop flag set directly (no notify) → the loop-top check exits.
        client.inner.stop.store(true, Ordering::SeqCst);
        tx.send(stream_event("ping", "{}", "")).unwrap();
        // Nothing observable to wait on here (and nothing asserted after it):
        // let the manager walk its exit path before the test drops it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.disconnect();
    }

    /// Drain the app event channel until an event of `kind` arrives, returning
    /// every type seen on the way (including `kind`); the test fails instead of
    /// hanging if it never does. The client forwards an event only after its
    /// run bookkeeping, so a received event proves the state it implies — this
    /// is what lets the tests below assert client state without sleeping.
    async fn recv_until(
        events: &mut mpsc::UnboundedReceiver<AgentEvent>,
        kind: &str,
    ) -> Vec<String> {
        match recv_until_within(events, kind, Duration::from_secs(10)).await {
            Some(seen) => seen,
            None => panic!("timed out waiting for a {kind:?} event"),
        }
    }

    /// [`recv_until`] without the failure: `None` when the event never arrives.
    async fn recv_until_within(
        events: &mut mpsc::UnboundedReceiver<AgentEvent>,
        kind: &str,
        timeout: Duration,
    ) -> Option<Vec<String>> {
        let mut seen = Vec::new();
        let arrived = tokio::time::timeout(timeout, async {
            while let Some(event) = events.recv().await {
                seen.push(event.r#type.clone());
                if event.r#type == kind {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        arrived.then_some(seen)
    }

    /// How long one attach probe may take before it counts as lost. The probe
    /// is an in-process gRPC round trip (milliseconds), so this is generous
    /// headroom for an instrumented, fully parallel test run.
    const ATTACH_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

    /// One attach probe: send a `ping` and report whether the client consumed it
    /// (a `ping` carries no run bookkeeping, so it is a pure liveness probe).
    async fn probe(source: &EventSource, events: &mut mpsc::UnboundedReceiver<AgentEvent>) -> bool {
        source.send(stream_event("ping", "{}", "")).is_ok()
            && recv_until_within(events, "ping", ATTACH_PROBE_TIMEOUT)
                .await
                .is_some()
    }

    /// Subscribe the client to the mock's stream and wait until it is provably
    /// consuming events.
    ///
    /// `set_current_session_id` and `connect_events` both poke the stream
    /// manager, and a poke that lands while it is subscribing makes it return
    /// `Poked` at the top of its loop — *before* it ever reads a message. So
    /// "a subscription was served" is no proof at all: an event sent to that
    /// subscription is dropped with it, which is what made these tests flaky
    /// (the old mock turned it into a `SendError` panic instead). Probe until an
    /// event actually comes back, and require two in a row: the first proves the
    /// subscription is reading, the second that it survived the poke still in
    /// flight behind it. Nothing pokes after this, so a subscription that
    /// answers both keeps receiving for the rest of the test.
    async fn attach(
        client: &GrpcClient,
        source: &EventSource,
        events: &mut mpsc::UnboundedReceiver<AgentEvent>,
    ) {
        client.set_current_session_id("s1");
        client.connect_events();
        for _ in 0..20 {
            if probe(source, events).await && probe(source, events).await {
                return;
            }
            // A probe can be lost to a resubscribe that was already in flight
            // (the manager serves the replacement within milliseconds): pause
            // and probe again. Deliberately not "wait for the next
            // subscription" — a probe that was merely slow must not leave the
            // test waiting on a resubscription that will never happen.
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("client never attached to a live event stream");
    }

    /// A poke while the subscription is live — the session-change +
    /// `connectEvents` pair the TUI issues when switching sessions — must leave
    /// the client on a working stream.
    #[tokio::test(flavor = "multi_thread")]
    async fn poke_during_live_subscription_resubscribes_without_losing_events() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, _conn) = GrpcClient::new(&addr);
        attach(&client, &tx, &mut events).await;

        // A poke against the live subscription, as the TUI's session switch
        // does: the manager resubscribes, and the new stream must be live.
        let before = tx.subscribe_count();
        client.connect_events();
        tx.wait_for_subscribes(before + 1).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
        recv_until(&mut events, "ping").await;
        client.disconnect();
    }

    #[test]
    fn parse_stream_event_maps_snapshot_events() {
        let ev = StreamEvent {
            r#type: "text_chunk".into(),
            data: "{\"text\":\"hi\"}".into(),
            projection_snapshot: true,
            snapshot_cursor: 7,
            snapshot_events: vec![
                future_rpc::proto::ProjectedRunEvent {
                    r#type: "agent_start".into(),
                    data: "{}".into(),
                    idx: 1,
                    ..Default::default()
                },
                future_rpc::proto::ProjectedRunEvent {
                    r#type: "agent_end".into(),
                    data: "{}".into(),
                    idx: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let raw: Map<String, Value> = serde_json::from_str("{\"text\":\"hi\"}").unwrap();
        let out = parse_stream_event(&ev, &raw);
        assert!(out.projection_snapshot);
        assert_eq!(out.snapshot_cursor, 7);
        assert_eq!(out.snapshot_events.len(), 2);
        assert_eq!(out.snapshot_events[0].r#type, "agent_start");
        assert_eq!(out.snapshot_events[1].idx, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unary_against_original_mock_agent() {
        let (_server, addr) = spawn_mock_agent().await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        assert!(client.try_connect().await);
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_unary_more_edges() {
        // Status with an empty message → rendered via to_string.
        let addr = spawn_api_mock(ApiMock {
            status_errors: StdHashMap::from([(
                "get_state".into(),
                tonic::Status::new(tonic::Code::Unknown, ""),
            )]),
            ..Default::default()
        })
        .await;
        let err = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Unknown"));

        // success=false with an empty error → "unknown error"; with an
        // error string → the string.
        let addr = spawn_api_mock(ApiMock {
            fail_with: StdHashMap::from([
                ("get_state".to_string(), String::new()),
                ("get_messages".to_string(), "boom".to_string()),
            ]),
            ..Default::default()
        })
        .await;
        let err = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "unknown error");
        let err = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_messages".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "boom");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repeated_get_state_same_agent_is_stable() {
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([(
                "get_state".to_string(),
                "{\"sessionId\":\"s1\",\"agentInstanceId\":\"agent-1\"}".to_string(),
            )]))),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.get_state().await.unwrap();
        client.get_state().await.unwrap(); // same instance id — no churn
        assert!(client.take_lost_queued_run_ids().is_empty());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn server_death_marks_disconnected_and_polls_reconnect() {
        let (server, addr) = spawn_mock_agent().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // Kill the server: the stream errors out → Lost → conn false, and
        // the reconnect poll runs tryConnect (fails while the server is down).
        server.abort();
        wait_conn(&mut conn, false, "disconnect notification").await;
        assert!(!client.is_connected());
        // Let the 1 s reconnect poll fire against the dead agent.
        tokio::time::sleep(Duration::from_millis(1_300)).await;
        assert!(!client.is_connected());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconnect_after_server_restart() {
        // Bind the mock, keep the address for a later re-bind.
        let (server, addr) = spawn_mock_agent().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        wait_connected(&mut conn).await;

        // Kill → disconnect notification.
        server.abort();
        wait_conn(&mut conn, false, "disconnect").await;
        assert!(!client.is_connected());

        // Revive on the same address: the poll's tryConnect succeeds →
        // resubscribe → connected again.
        // Revive on the same address — bind it here and keep the listener, so
        // the port cannot be taken between the bind and the serve.
        let listener = tokio::net::TcpListener::bind(addr.as_str()).await.unwrap();
        let addr2 = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        assert_eq!(addr2.to_string(), addr);
        let agent = MockAgent {
            event_tx: Arc::new(tokio::sync::Mutex::new(None)),
        };
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve_with_incoming(incoming),
        );
        wait_conn(&mut conn, true, "reconnect").await;
        assert!(client.is_connected());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconnect_poll_session_change_and_stop() {
        // Dead agent → the manager sits in the reconnect poll.
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!client.is_connected());

        // Session change WITHOUT a poke (white-box): the poll's pre-select
        // check breaks out to resubscribe.
        client.inner.state.lock().current_session_id = "s2".into();
        tokio::time::sleep(Duration::from_millis(1_300)).await;

        // A poke wakes the poll's select; the post-select check breaks.
        client.set_current_session_id("s3");
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Stop without notify → the poll's loop-top check exits.
        client.inner.stop.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(1_300)).await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconnect_poll_pre_select_session_change() {
        // A black-hole agent: accepts TCP, never answers. The session change
        // lands while a subscribe/tryConnect is blocked mid-flight, so the
        // reconnect poll's pre-select session check catches it.
        // Bind here and move the listener into the task: the port is held
        // from the start, so nothing can steal it before it starts accepting.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            // Sockets are held open (never speaking) until the runtime ends.
            let mut held = Vec::new();
            loop {
                let (sock, _) = listener.accept().await.unwrap();
                held.push(sock);
            }
        });
        let addr = format!("127.0.0.1:{}", addr.port());

        let (client, _e, _c) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        // The subscription hangs (no data). The 5 s watchdog ends it; the
        // manager enters the reconnect poll, and its 1 s tick starts a
        // (black-hole-slow) tryConnect — the black hole holds the TCP
        // handshake, so it times out at 3 s.
        tokio::time::sleep(Duration::from_millis(6_500)).await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconnect_poll_pre_select_check_fires() {
        // Slow-failing agent: subscribe errors fast (Lost), then each
        // tryConnect takes ~1 s to fail. Change the session while that call
        // is in flight — the poll's pre-select check resubscribes.
        let addr = spawn_api_mock(ApiMock {
            stream_fails: true,
            unary_delay_ms: 800,
            fail_with: StdHashMap::from([("list_models".to_string(), String::new())]),
            ..Default::default()
        })
        .await;
        let (client, _e, _c) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        // Lost quickly (stream error) → poll; tick at ≈1 s starts the slow
        // tryConnect; land the session change inside that window.
        tokio::time::sleep(Duration::from_millis(1_300)).await;
        client.inner.state.lock().current_session_id = "s2".into();
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn manager_loop_top_stop_with_idle_manager() {
        // Manager idle (no session) + stop without notify + a poke to wake
        // it → the loop-top stop check returns.
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.inner.stop.store(true, Ordering::SeqCst);
        client.connect_events(); // poke
        tokio::time::sleep(Duration::from_millis(200)).await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_stream_error_is_lost() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, mut conn) = GrpcClient::new(&addr);
        attach(&client, &tx, &mut events).await;
        wait_connected(&mut conn).await;
        // A stream-level error → Lost → disconnect notification.
        tx.send(Err(tonic::Status::internal("mid-stream boom")))
            .unwrap();
        wait_conn(&mut conn, false, "disconnect on stream error").await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_failure_modes() {
        // Bad endpoint (unparseable addr).
        let (client, _e, _c) = GrpcClient::new("bad addr with spaces");
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!client.is_connected());
        client.disconnect();

        // Connect failure (nothing listening).
        let (client, _e, _c) = GrpcClient::new("127.0.0.1:1");
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!client.is_connected());
        client.disconnect();

        // stream_events errors on the server side.
        let addr = spawn_api_mock(ApiMock {
            stream_fails: true,
            ..Default::default()
        })
        .await;
        let (client, _e, _c) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!client.is_connected());
        client.disconnect();

        // Stream ends immediately (after one event).
        let addr = spawn_api_mock(ApiMock {
            stream_ends: true,
            ..Default::default()
        })
        .await;
        let (client, _e, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        wait_connected(&mut conn).await;
        // The end of the stream flips the connection back off.
        wait_conn(&mut conn, false, "stream-end disconnect").await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn watchdog_fires_on_dataless_stream() {
        // A subscription with no events at all → the 5 s watchdog fires.
        let addr = spawn_api_mock(ApiMock {
            stream_idle: true,
            ..Default::default()
        })
        .await;
        let (client, _e, _c) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        // Watchdog is 5 s; wait past it.
        tokio::time::sleep(Duration::from_millis(5_500)).await;
        assert!(!client.is_connected());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn heartbeat_marks_dead_agent_disconnected() {
        // Tests run the heartbeat at 50 ms (cfg(test) HEARTBEAT_MS).
        let (server, addr) = spawn_mock_agent().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // Kill the agent: the next heartbeat's tryConnect fails → disconnect.
        server.abort();
        wait_conn(&mut conn, false, "heartbeat disconnect").await;
        assert!(!client.is_connected());
        client.disconnect();
    }

    #[tokio::test]
    async fn wait_connected_helper_loops_until_true() {
        let (tx, mut rx) = watch::channel(false);
        let flipper = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send(true);
        });
        wait_connected(&mut rx).await;
        flipper.abort();
        // Already-connected resolves immediately.
        let (_tx, mut rx) = watch::channel(true);
        wait_connected(&mut rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_state_shape_error_and_bad_endpoint() {
        // get_state with a JSON shape that fails RpcSessionState
        // deserialization → the map_err arm.
        let addr = spawn_api_mock(ApiMock {
            data_by_type: Arc::new(std::sync::Mutex::new(StdHashMap::from([(
                "get_state".to_string(),
                r#"{"queuedRuns":"not-an-array"}"#.to_string(),
            )]))),
            ..Default::default()
        })
        .await;
        let (client, _e, _c) = GrpcClient::new(&addr);
        assert!(client.get_state().await.is_err());
        client.disconnect();

        // execute_unary with an unparseable address → from_shared arm.
        assert!(
            execute_unary("bad addr with spaces", RpcCommand::default(), 1)
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poked_exit_with_stop_set_returns_manager() {
        // Latch stop WITHOUT notifying, then change the session (poke): the
        // stale subscription exits Poked and the manager's post-Poked stop
        // check returns instead of resubscribing.
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, mut conn) = GrpcClient::new(&addr);
        attach(&client, &tx, &mut events).await;
        wait_connected(&mut conn).await;
        client.inner.stop.store(true, Ordering::SeqCst);
        client.set_current_session_id("s2"); // poke → Poked → stop → return
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.disconnect();
    }
}
