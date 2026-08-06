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

use crate::generated::proto::future_agent_client::FutureAgentClient;
use crate::generated::proto::{RpcCommand, StreamEvent, StreamRequest};
use crate::rpc::types::{
    AgentEvent, ModelInfo, ProjectedRunEvent, RpcSessionState, RunAck, SessionSummary,
};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch, Notify};
use tonic::transport::Endpoint;
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
/// Heartbeat interval (silent-disconnection detection).
const HEARTBEAT_MS: u64 = 10_000;
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

/// `grpcAddr()` — env override, then localhost default.
pub fn grpc_addr() -> String {
    std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "localhost:50051".to_string())
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
                runs: HashMap::new(),
                agent_instance_id: None,
                lost_queued_run_ids: Vec::new(),
            }),
            event_tx,
            conn_tx,
            poke_count: AtomicU64::new(0),
            poke_notify: Notify::new(),
            stop: AtomicBool::new(false),
            stop_notify: Notify::new(),
        });

        // Stream manager: subscribes to the current session's event stream,
        // reconnects on failure (1 s tryConnect polling), restarts when the
        // session changes or `connect_events` is called.
        spawn_stream_manager(inner.clone());
        // Heartbeat: detects silent disconnections (agent SIGKILL'd).
        spawn_heartbeat(inner.clone());

        (GrpcClient { inner }, events, connection_changes)
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
    /// payload (raw string passthrough on parse failure, `Null` when empty).
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
            if cmd.session_id.is_empty() && !st.current_session_id.is_empty() {
                cmd.session_id = st.current_session_id.clone();
            }
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
        let timeout = tokio::time::sleep(Duration::from_millis(max_ms));
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                changed = rx.changed() => {
                    match changed {
                        Ok(()) => {
                            if *rx.borrow() { return; }
                        }
                        Err(_) => return,
                    }
                }
                _ = &mut timeout => return,
            }
        }
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
    pub async fn prompt(&self, message: &str, busy_policy: &str) -> Result<RunAck, String> {
        let request_id = uuid_hex();
        let cmd = RpcCommand {
            message: message.to_string(),
            requested_run_id: format!("run_{}", uuid_hex()),
            client_request_id: format!("request_{request_id}"),
            busy_policy: busy_policy.to_string(),
            ..Default::default()
        };
        let resp = self.call("prompt", cmd).await?;
        let ack: RunAck = serde_json::from_value(resp).map_err(|e| e.to_string())?;
        let mut st = self.inner.state.lock();
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

    /// `reloadConfig()` — `{skills, contextFiles}`.
    pub async fn reload_config(&self) -> Result<Value, String> {
        self.call("reload_config", RpcCommand::default()).await
    }
}

// ─── Unary RPC ─────────────────────────────────────────────────────────────

/// One-shot `ExecuteCommand` with a deadline (mirrors the CLI port; the TS
/// client reuses a channel, per-call connect is equivalent for our use).
async fn execute_unary(addr: &str, cmd: RpcCommand, timeout_secs: u64) -> Result<Value, String> {
    let endpoint = Endpoint::from_shared(format!("http://{addr}"))
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(timeout_secs));
    let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
    let mut client = FutureAgentClient::new(channel);
    let response = client
        .execute_command(cmd)
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

    if response.data.is_empty() {
        return Ok(Value::Null);
    }
    match serde_json::from_str::<Value>(&response.data) {
        Ok(value) => Ok(value),
        Err(_) => Ok(Value::String(response.data)),
    }
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
            if inner.stop.load(Ordering::SeqCst) {
                return;
            }

            let session = inner.state.lock().current_session_id.clone();
            if session.is_empty() {
                // Never subscribe without a session ID — an empty session_id
                // may leak events from ALL sessions (TS comment). Wait for a
                // poke or stop.
                tokio::select! {
                    _ = inner.poke_notify.notified() => {}
                    _ = inner.stop_notify.notified() => {
                        if inner.stop.load(Ordering::SeqCst) { return; }
                    }
                }
                continue;
            }

            // Subscribe until the stream ends, errors, the 5 s first-data
            // watchdog fires, or the session changes.
            let connected = subscribe_stream(&inner, &session).await;
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
            let _ = connected; // the TS "connected" edge only matters for call()

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

/// Subscribe to the session's event stream; returns true if at least one
/// event was delivered (the TS `connected` edge). Loops until the stream
/// ends, errors, the 5 s first-data watchdog fires, the session changes, or
/// a poke arrives.
async fn subscribe_stream(inner: &Arc<Inner>, session: &str) -> bool {
    let endpoint = match Endpoint::from_shared(format!("http://{}", inner.addr)) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let channel = match endpoint.connect().await {
        Ok(c) => c,
        Err(_) => return false,
    };
    let mut client = FutureAgentClient::new(channel);
    let request = StreamRequest {
        session_id: session.to_string(),
        ..Default::default()
    };
    let mut stream = match client.stream_events(request).await {
        Ok(resp) => resp.into_inner(),
        Err(_) => return false,
    };

    let mut connected = false;
    let watchdog = tokio::time::sleep(Duration::from_millis(CONNECT_WATCHDOG_MS));
    tokio::pin!(watchdog);
    loop {
        // Session changed or stopped — exit so the caller resubscribes.
        if inner.state.lock().current_session_id != session {
            return connected;
        }
        if inner.stop.load(Ordering::SeqCst) {
            return connected;
        }

        tokio::select! {
            msg = stream.message() => {
                match msg {
                    Ok(Some(event)) => {
                        // Data arrived — restart the watchdog.
                        watchdog
                            .as_mut()
                            .reset(tokio::time::Instant::now() + Duration::from_millis(CONNECT_WATCHDOG_MS));
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
                            return connected; // app gone
                        }
                    }
                    Ok(None) => return connected, // stream end
                    Err(_) => return connected,   // stream error
                }
            }
            _ = &mut watchdog => {
                // No data within 5 s — the channel is likely stuck. Cancel
                // and let the caller run the tryConnect reconnect poll.
                return connected;
            }
            _ = inner.poke_notify.notified() => {
                // Session change or explicit connect_events — resubscribe.
                return connected;
            }
            _ = inner.stop_notify.notified() => {
                if inner.stop.load(Ordering::SeqCst) { return connected; }
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
}
