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

use crate::rpc::types::{
    AgentEvent, ModelInfo, ProjectedRunEvent, RpcSessionState, RunAck, SessionSummary,
};
use future_rpc::proto::future_agent_client::FutureAgentClient;
use future_rpc::proto::{RpcCommand, StreamEvent, StreamRequest};
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
    let endpoint = match Endpoint::from_shared(format!("http://{}", inner.addr)) {
        Ok(e) => e,
        Err(_) => return StreamExit::Lost,
    };
    let channel = match endpoint.connect().await {
        Ok(c) => c,
        Err(_) => return StreamExit::Lost,
    };
    let mut client = FutureAgentClient::new(channel);
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
            _ = inner.poke_notify.notified() => {
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
    use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};
    use futures_util::stream;
    use futures_util::StreamExt;
    use std::net::TcpListener;
    use std::pin::Pin;
    use tokio_stream::wrappers::UnboundedReceiverStream;
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // tonic binds the same port below
        let agent = MockAgent {
            event_tx: Arc::new(tokio::sync::Mutex::new(None)),
        };
        // Spawn the serve future directly — no async-block tail that can
        // never complete.
        let handle = tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve(addr),
        );
        // Give the server a moment to start listening.
        tokio::time::sleep(Duration::from_millis(50)).await;
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
        status_errors: StdHashMap<String, tonic::Status>,
        seen: Arc<std::sync::Mutex<Vec<String>>>,
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
            self.seen.lock().unwrap().push(cmd.r#type.clone());
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
                payload: None,
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

    async fn spawn_api_mock(mock: ApiMock) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        // Spawn the serve future directly (no async block → no never-taken
        // completion tail).
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(mock))
                .serve(addr),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        format!("127.0.0.1:{}", addr.port())
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
        assert_eq!(grpc_addr(), "localhost:50051");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "example:1234");
        assert_eq!(grpc_addr(), "example:1234");
        // Both restore arms.
        restore("FUTURE_AGENT_GRPC_ADDR", Some("ambient".into()));
        assert_eq!(grpc_addr(), "ambient");
        restore("FUTURE_AGENT_GRPC_ADDR", None);
        assert_eq!(grpc_addr(), "localhost:50051");
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
                ("compact".into(), "{\"summary\":\"shorter\"}".into()),
                ("reload_config".into(), "{\"reloaded\":true}".into()),
            ]))),
            ..Default::default()
        })
        .await;
        let (client, _events, _conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");

        // prompt (running) → run tracked as active.
        let ack = client.prompt("hello", "queue").await.unwrap();
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

        let ack = client.prompt("later", "queue").await.unwrap();
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
        client.prompt("again", "queue").await.unwrap();
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

        // Empty data → Null; non-JSON data → String.
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
        assert_eq!(v, Value::String("raw text".into()));
    }

    /// Mock whose event stream is fed by a test-owned channel of Results
    /// (so tests can inject stream errors).
    type SharedEventRx = Arc<
        tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<Result<StreamEvent, tonic::Status>>>>,
    >;

    #[derive(Clone)]
    struct EventfulMock {
        rx: SharedEventRx,
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
            let rx = self.rx.lock().await.take();
            match rx {
                Some(rx) => {
                    let stream = UnboundedReceiverStream::new(rx);
                    Ok(tonic::Response::new(Box::pin(stream)))
                }
                // Only one subscription gets the channel; resubscribes idle.
                None => Ok(tonic::Response::new(Box::pin(stream::pending()))),
            }
        }
    }

    async fn spawn_eventful_mock() -> (
        mpsc::UnboundedSender<Result<StreamEvent, tonic::Status>>,
        String,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<Result<StreamEvent, tonic::Status>>();
        let shared_rx = Arc::new(tokio::sync::Mutex::new(Some(rx)));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let mock = EventfulMock { rx: shared_rx };
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(mock))
                .serve(addr),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        (tx, format!("127.0.0.1:{}", addr.port()))
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

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_stream_event_bookkeeping() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, mut events, _conn) = GrpcClient::new(&addr);
        // A unary call exercises the mock's execute_command.
        assert!(client.try_connect().await);
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Malformed data is dropped without disturbing the stream.
        tx.send(stream_event("text_chunk", "not json", "")).unwrap();
        // A run-scoped event that is neither start nor end skips the
        // bookkeeping arms.
        tx.send(stream_event("text_chunk", "{\"text\":\"x\"}", "r0"))
            .unwrap();
        // agent_start marks the run active…
        tx.send(stream_event("agent_start", "{}", "r1")).unwrap();
        assert!(spin_until_bool(&client, true).await);
        assert!(client.has_running_run());
        // agent_end for a DIFFERENT run keeps the active run…
        tx.send(stream_event("agent_end", "{}", "r9")).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(client.has_running_run());
        // …and agent_end for the active run clears it.
        tx.send(stream_event("agent_end", "{}", "r1")).unwrap();
        assert!(spin_until_bool(&client, false).await);
        assert!(!client.has_running_run());
        // The events also flow to the app channel.
        let mut received = Vec::new();
        while let Ok(ev) = events.try_recv() {
            received.push(ev.r#type.clone());
        }
        assert!(received.contains(&"agent_start".to_string()));
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_loop_top_session_check() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        // The first delivered event flips the connection on.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // Session changed WITHOUT a poke (white-box): the next event drives
        // the loop iteration whose top check silently resubscribes.
        client.inner.state.lock().current_session_id = "s2".into();
        tx.send(stream_event("ping", "{}", "")).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_event_channel_closed_is_lost() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // App event channel dropped → the next event send fails → Lost.
        drop(events);
        tx.send(stream_event("ping", "{}", "")).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!client.is_connected());
        client.disconnect();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscribe_loop_top_stop_check() {
        let (tx, addr) = spawn_eventful_mock().await;
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
        wait_connected(&mut conn).await;
        assert!(client.is_connected());

        // Stop flag set directly (no notify) → the loop-top check exits.
        client.inner.stop.store(true, Ordering::SeqCst);
        tx.send(stream_event("ping", "{}", "")).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.disconnect();
    }

    /// Spin until the client's active-run state matches `want`.
    async fn spin_until_bool(client: &GrpcClient, want: bool) -> bool {
        for _ in 0..200 {
            if client.has_running_run() == want {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
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
    async fn spin_until_bool_times_out() {
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        // has_running_run never becomes true → the helper times out.
        assert!(!spin_until_bool(&client, true).await);
        client.disconnect();
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
        let listener = TcpListener::bind(&addr).unwrap();
        let addr2 = listener.local_addr().unwrap();
        drop(listener);
        let agent = MockAgent {
            event_tx: Arc::new(tokio::sync::Mutex::new(None)),
        };
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve(addr2),
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            // Sockets are held open (never speaking) until the runtime ends.
            let mut held = Vec::new();
            loop {
                let (sock, _) = listener.accept().await.unwrap();
                held.push(sock);
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
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
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
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
        let (client, _events, mut conn) = GrpcClient::new(&addr);
        client.set_current_session_id("s1");
        client.connect_events();
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(stream_event("ping", "{}", "")).unwrap();
        wait_connected(&mut conn).await;
        client.inner.stop.store(true, Ordering::SeqCst);
        client.set_current_session_id("s2"); // poke → Poked → stop → return
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.disconnect();
    }
}
