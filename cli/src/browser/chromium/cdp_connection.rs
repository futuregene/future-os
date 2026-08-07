//! CDP connection over WebSocket — port of
//! `cli/src/browser/chromium/cdp-connection.ts`.
//!
//! Responsibilities:
//! - Incrementing request IDs and promise matching
//! - Per-request timeout
//! - Session management (CdpSession handles per-target command dispatch)
//! - Pending request rejection on close
//! - TargetSessionRegistry for targetId ↔ sessionId mapping

use super::cdp_event_router::{CdpEventRouter, Unsubscribe};
use super::cdp_transport::{TransportEvent, WebSocketTransport};
use super::target_registry::{AttachedTarget, SharedTargetRegistry, TargetSessionRegistry};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

// ── Errors ───────────────────────────────────────────────────────────

/// `CdpError` — protocol error carrying the CDP code.
#[derive(Debug, Clone)]
pub struct CdpError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `CdpConnectionError` — "Connection is closed".
#[derive(Debug, Clone)]
pub struct CdpConnectionError(pub String);

impl std::fmt::Display for CdpConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `CdpTimeoutError` — `CDP command "<method>" timed out after <ms>ms`.
#[derive(Debug, Clone)]
pub struct CdpTimeoutError {
    pub method: String,
    pub timeout_ms: u64,
}

impl std::fmt::Display for CdpTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CDP command \"{}\" timed out after {}ms",
            self.method, self.timeout_ms
        )
    }
}

/// Unified error type surfaced by `send`.
#[derive(Debug, Clone)]
pub enum CdpSendError {
    Protocol(CdpError),
    Timeout(CdpTimeoutError),
    Closed,
}

impl std::fmt::Display for CdpSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CdpSendError::Protocol(e) => e.fmt(f),
            CdpSendError::Timeout(e) => e.fmt(f),
            CdpSendError::Closed => f.write_str("Connection is closed"),
        }
    }
}

// ── Pending request ──────────────────────────────────────────────────

struct PendingRequest {
    session_id: Option<String>,
    tx: oneshot::Sender<Result<Value, CdpSendError>>,
}

// ── CdpConnection ────────────────────────────────────────────────────

/// `CdpConnection` — shared (Arc) connection with a dispatch task fanning
/// inbound messages to pending responses and the event router.
pub struct CdpConnection {
    transport: Arc<WebSocketTransport>,
    request_id: AtomicU64,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    event_router: Arc<CdpEventRouter>,
    target_registry: SharedTargetRegistry,
    default_timeout_ms: u64,
    connected: AtomicBool,
}

impl CdpConnection {
    /// Connect to a CDP WebSocket endpoint and start the dispatch loop.
    pub async fn connect(
        web_socket_debugger_url: &str,
        timeout_ms: u64,
    ) -> Result<Arc<Self>, String> {
        let transport =
            Arc::new(WebSocketTransport::connect(web_socket_debugger_url, timeout_ms).await?);
        let conn = Arc::new(CdpConnection {
            transport,
            request_id: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            event_router: Arc::new(CdpEventRouter::new()),
            target_registry: Arc::new(Mutex::new(TargetSessionRegistry::new())),
            default_timeout_ms: 10_000,
            connected: AtomicBool::new(true),
        });

        // Dispatch loop: owns a clone of the connection, forwards transport
        // events to handle_message / handle_close.
        let dispatch = conn.clone();
        tokio::spawn(async move {
            let mut rx = dispatch.transport.subscribe();
            loop {
                match rx.recv().await {
                    Ok(TransportEvent::Message(raw)) => dispatch.handle_message(&raw),
                    Ok(TransportEvent::Close(reason)) => dispatch.handle_close(reason),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(conn)
    }

    // ── Send ──────────────────────────────────────────────────────────

    /// Send a CDP command and wait for the response (default timeout).
    pub async fn send(
        &self,
        method: &str,
        params: Option<&Map<String, Value>>,
        session_id: Option<&str>,
    ) -> Result<Value, CdpSendError> {
        self.send_with_timeout(method, params, session_id, self.default_timeout_ms)
            .await
    }

    /// Send a CDP command with an explicit timeout.
    pub async fn send_with_timeout(
        &self,
        method: &str,
        params: Option<&Map<String, Value>>,
        session_id: Option<&str>,
        timeout_ms: u64,
    ) -> Result<Value, CdpSendError> {
        if !self.is_connected() {
            return Err(CdpSendError::Closed);
        }

        let id = self.request_id.fetch_add(1, Ordering::SeqCst) + 1;

        let mut message = Map::new();
        message.insert("id".to_string(), Value::Number(id.into()));
        message.insert("method".to_string(), Value::String(method.to_string()));
        if let Some(params) = params {
            message.insert("params".to_string(), Value::Object(params.clone()));
        }
        if let Some(sid) = session_id {
            message.insert("sessionId".to_string(), Value::String(sid.to_string()));
        }

        let (tx, rx) = oneshot::channel::<Result<Value, CdpSendError>>();
        self.pending.lock().unwrap().insert(
            id,
            PendingRequest {
                session_id: session_id.map(str::to_string),
                tx,
            },
        );
        self.transport
            .send(&serde_json::to_string(&message).unwrap_or_default());

        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(CdpSendError::Closed),
            Err(_) => {
                // Remove the timed-out pending request (TS clearTimeout + delete).
                self.pending.lock().unwrap().remove(&id);
                Err(CdpSendError::Timeout(CdpTimeoutError {
                    method: method.to_string(),
                    timeout_ms,
                }))
            }
        }
    }

    // ── Events ────────────────────────────────────────────────────────

    /// Register a handler for (sessionId, method). Returns unsubscribe.
    pub fn on(
        &self,
        session_id: Option<&str>,
        method: &str,
        handler: super::cdp_event_router::CdpEventHandler,
    ) -> Unsubscribe {
        self.event_router.add(session_id, method, handler)
    }

    // ── Sessions ──────────────────────────────────────────────────────

    // Sessions are created via `CdpSession::new(sessionId, connection)`
    // (the TS `createSession` returns a fresh `new CdpSession(sessionId, this)`).

    // ── Target registry ───────────────────────────────────────────────

    pub fn register_target(&self, target: AttachedTarget) {
        self.target_registry.lock().unwrap().add(target);
    }

    pub fn detach_target_by_session_id(&self, session_id: &str) -> Option<AttachedTarget> {
        self.target_registry
            .lock()
            .unwrap()
            .detach_by_session_id(session_id)
    }

    pub fn detach_target_by_target_id(&self, target_id: &str) -> Option<AttachedTarget> {
        self.target_registry
            .lock()
            .unwrap()
            .detach_by_target_id(target_id)
    }

    pub fn get_target_by_session_id(&self, session_id: &str) -> Option<AttachedTarget> {
        self.target_registry
            .lock()
            .unwrap()
            .get_by_session_id(session_id)
            .cloned()
    }

    // ── Pending cleanup ───────────────────────────────────────────────

    /// Reject all pending requests for a session (target detached/destroyed).
    pub fn reject_pending_for_session(&self, session_id: &str, error: CdpSendError) {
        let mut pending = self.pending.lock().unwrap();
        let ids: Vec<u64> = pending
            .iter()
            .filter(|(_, p)| p.session_id.as_deref() == Some(session_id))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(p) = pending.remove(&id) {
                let _ = p.tx.send(Err(error.clone()));
            }
        }
    }

    // ── Disconnect ────────────────────────────────────────────────────

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Test-only: drive an event straight into the router (no transport).
    #[cfg(test)]
    pub fn dispatch_test(&self, session_id: Option<&str>, method: &str, params: &Value) {
        self.event_router.dispatch(session_id, method, params);
    }

    pub async fn disconnect(&self) {
        if !self.is_connected() {
            return;
        }
        self.connected.store(false, Ordering::SeqCst);

        // Reject all pending requests.
        let close_error = CdpSendError::Protocol(CdpError {
            code: -1,
            message: "Connection closed".to_string(),
        });
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for (_id, p) in pending {
            let _ = p.tx.send(Err(close_error.clone()));
        }

        // Clean up event handlers.
        self.event_router.clear();

        // Close transport (bounded).
        self.transport.close().await;
    }

    // ── Internal ──────────────────────────────────────────────────────

    fn handle_message(&self, raw: &str) {
        let message: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => return, // Malformed — ignore
        };
        let Some(message) = message.as_object() else {
            return;
        };

        let id = message.get("id").and_then(Value::as_u64);
        let session_id = message.get("sessionId").and_then(Value::as_str);

        // Response to a pending request (has id, no method).
        if let Some(id) = id {
            if message.get("method").is_some() {
                return;
            }
            let pending = self.pending.lock().unwrap().remove(&id);
            let Some(pending) = pending else {
                return; // Stale response
            };
            if let Some(error) = message.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
                let msg = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown CDP error")
                    .to_string();
                let _ = pending
                    .tx
                    .send(Err(CdpSendError::Protocol(CdpError { code, message: msg })));
            } else {
                let _ = pending
                    .tx
                    .send(Ok(message.get("result").cloned().unwrap_or(Value::Null)));
            }
            return;
        }

        // Event.
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            self.event_router.dispatch(
                session_id,
                method,
                message.get("params").unwrap_or(&Value::Null),
            );
        }
    }

    fn handle_close(&self, _reason: Option<String>) {
        let error = CdpSendError::Protocol(CdpError {
            code: -1,
            message: "Connection closed".to_string(),
        });
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for (_id, p) in pending {
            let _ = p.tx.send(Err(error.clone()));
        }
        self.connected.store(false, Ordering::SeqCst);
    }
}

// ── CdpSession ───────────────────────────────────────────────────────

/// `CdpSession` — per-target (or browser-level, session_id "") command
/// dispatch. Clones share the underlying connection.
#[derive(Clone)]
pub struct CdpSession {
    pub session_id: String,
    connection: Arc<CdpConnection>,
}

impl CdpSession {
    pub fn new(session_id: &str, connection: Arc<CdpConnection>) -> Self {
        CdpSession {
            session_id: session_id.to_string(),
            connection,
        }
    }

    pub fn connection(&self) -> &Arc<CdpConnection> {
        &self.connection
    }

    pub async fn send(
        &self,
        method: &str,
        params: Option<&Map<String, Value>>,
    ) -> Result<Value, CdpSendError> {
        self.connection
            .send(method, params, Some(&self.session_id))
            .await
    }

    pub async fn send_with_timeout(
        &self,
        method: &str,
        params: Option<&Map<String, Value>>,
        timeout_ms: u64,
    ) -> Result<Value, CdpSendError> {
        self.connection
            .send_with_timeout(method, params, Some(&self.session_id), timeout_ms)
            .await
    }

    /// `on(method, handler)` — normalized: browser-level session (empty
    /// string) → undefined sessionId.
    pub fn on(
        &self,
        method: &str,
        handler: super::cdp_event_router::CdpEventHandler,
    ) -> Unsubscribe {
        let sid = if self.session_id.is_empty() {
            None
        } else {
            Some(self.session_id.as_str())
        };
        self.connection.on(sid, method, handler)
    }

    /// Listen for browser-level events (not scoped to this session).
    pub fn on_browser(
        &self,
        method: &str,
        handler: super::cdp_event_router::CdpEventHandler,
    ) -> Unsubscribe {
        self.connection.on(None, method, handler)
    }
}
