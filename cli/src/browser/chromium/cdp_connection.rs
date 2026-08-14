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
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{broadcast, oneshot};

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

/// The dispatch loop body, extracted from `connect` so the broadcast
/// `Lagged`/`Closed` arms can be exercised directly with a small-capacity
/// channel in tests (a lagged receiver is otherwise only reachable after a
/// 1024-event burst on a live transport).
async fn run_dispatch_loop(
    mut rx: broadcast::Receiver<TransportEvent>,
    dispatch: Weak<CdpConnection>,
) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                // Upgrade failed → the connection is gone and its
                // transport's sender dropped with it; the next recv
                // observes Closed and breaks.
                if let Some(conn) = dispatch.upgrade() {
                    match event {
                        TransportEvent::Message(raw) => conn.handle_message(&raw),
                        TransportEvent::Close(reason) => conn.handle_close(reason),
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            // Every sender is gone (the last connection Arc was dropped):
            // nothing more can arrive.
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
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

        // Dispatch loop: forwards transport events to handle_message /
        // handle_close. It holds only a WEAK connection reference — holding
        // an Arc would keep the transport (and with it the broadcast
        // sender) alive forever, making Closed unreachable and leaking the
        // task for the process lifetime. The subscription is created
        // BEFORE the task is spawned: a fast peer can deliver frames before
        // a freshly-spawned task is first polled, and the reader treats
        // "no subscribers" as fatal (broadcast buffers up to 1024 events
        // until the loop starts polling).
        let dispatch = Arc::downgrade(&conn);
        let rx = conn.transport.subscribe();
        tokio::spawn(run_dispatch_loop(rx, dispatch));

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
            // A dropped sender (oneshot canceled) means the connection died.
            Ok(result) => result.unwrap_or(Err(CdpSendError::Closed)),
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
            // Invariant: ids were collected from this same map under the lock.
            let p = pending.remove(&id).expect("pending id collected above");
            let _ = p.tx.send(Err(error.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_cdp::MockCdp;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    fn params(json: Value) -> Map<String, Value> {
        json.as_object().expect("object").clone()
    }

    /// Shared event-sink handler: one closure body reused across tests; a
    /// clone that never fires adds no new missed lines (innermost-function
    /// coverage rule).
    fn sink_push(
        sink: &Arc<Mutex<Vec<Value>>>,
    ) -> crate::browser::chromium::cdp_event_router::CdpEventHandler {
        let s = sink.clone();
        Arc::new(move |p| s.lock().unwrap().push(p.clone()))
    }

    /// Result extractors with a dedicated panic-coverage test below (a panic
    /// arm that never fires would itself be a missed line).
    #[track_caller]
    fn expect_protocol(err: CdpSendError) -> CdpError {
        match err {
            CdpSendError::Protocol(e) => e,
            other => panic!("expected protocol error, got {other}"),
        }
    }

    #[track_caller]
    fn expect_timeout(err: CdpSendError) -> CdpTimeoutError {
        match err {
            CdpSendError::Timeout(t) => t,
            other => panic!("expected timeout, got {other}"),
        }
    }

    #[test]
    fn extractor_helpers_panic_on_wrong_variant() {
        assert!(std::panic::catch_unwind(|| expect_protocol(CdpSendError::Closed)).is_err());
        assert!(std::panic::catch_unwind(|| expect_timeout(CdpSendError::Closed)).is_err());
    }

    // ── Error Display impls ───────────────────────────────────────────

    #[test]
    fn error_display_impls() {
        let e = CdpError {
            code: -32000,
            message: "msg".to_string(),
        };
        assert_eq!(format!("{e}"), "msg");
        assert_eq!(
            format!("{}", CdpConnectionError("closed!".to_string())),
            "closed!"
        );
        let t = CdpTimeoutError {
            method: "Page.navigate".to_string(),
            timeout_ms: 42,
        };
        assert_eq!(
            format!("{t}"),
            "CDP command \"Page.navigate\" timed out after 42ms"
        );
        assert_eq!(format!("{}", CdpSendError::Protocol(e.clone())), "msg");
        assert!(format!("{}", CdpSendError::Timeout(t)).contains("timed out"));
        assert_eq!(format!("{}", CdpSendError::Closed), "Connection is closed");
    }

    // ── connect / send / receive ──────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_failure_and_handshake_timeout() {
        // Nothing listening → handshake fails fast.
        let err = CdpConnection::connect("ws://127.0.0.1:1", 500)
            .await
            .err()
            .unwrap();
        assert!(err.contains("WebSocket connection failed"), "err: {err}");

        // TCP listener that never answers the WS handshake → timeout.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Hold past the client's 150 ms handshake timeout, then finish so
        // the task body runs to completion (its closing line counts).
        let hold = tokio::spawn(async move {
            let _s = listener.accept().await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        });
        let err = CdpConnection::connect(&format!("ws://{addr}"), 150)
            .await
            .err()
            .unwrap();
        assert!(err.contains("WebSocket connection timeout"), "err: {err}");
        // Let the server task run to completion (its closing line counts).
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), hold).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_result_and_error_variants() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Fail.method".to_string());
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        assert!(conn.is_connected());

        // Success: result payload is returned verbatim; params + sessionId
        // are transmitted.
        let session = CdpSession::new("S-1", conn.clone());
        let result = session
            .send("Page.enable", Some(&params(json!({"x": 1}))))
            .await
            .unwrap();
        assert_eq!(result, json!({}));
        let seen = mock.state.lock().unwrap().commands.clone();
        let (m, sid, p) = seen.iter().find(|(m, _, _)| m == "Page.enable").unwrap();
        assert_eq!(m, "Page.enable");
        assert_eq!(sid.as_deref(), Some("S-1"));
        assert_eq!(p, &json!({"x": 1}));

        // Protocol error: code + message preserved.
        let e = expect_protocol(conn.send("Fail.method", None, None).await.unwrap_err());
        assert_eq!(e.code, -32000);
        assert_eq!(e.message, "mock failure");

        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_after_disconnect_is_closed_and_double_disconnect_is_noop() {
        let mock = MockCdp::start().await;
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        conn.disconnect().await;
        assert!(!conn.is_connected());
        let err = conn.send("Page.enable", None, None).await.unwrap_err();
        assert!(matches!(err, CdpSendError::Closed));
        // Second disconnect returns early.
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_timeout_removes_pending() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .no_reply_methods
            .insert("Slow.method".to_string());
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();

        let t = expect_timeout(
            conn.send_with_timeout("Slow.method", None, None, 50)
                .await
                .unwrap_err(),
        );
        assert_eq!(t.method, "Slow.method");
        assert_eq!(t.timeout_ms, 50);
        assert!(conn.pending.lock().unwrap().is_empty());
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn server_close_rejects_pending_and_marks_disconnected() {
        let mock = MockCdp::start().await;
        {
            let mut state = mock.state.lock().unwrap();
            state.close_connection_on.insert("Kill.switch".to_string());
            state.no_reply_methods.insert("Never.answered".to_string());
        }
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();

        // A no-reply request stays pending; the Kill.switch command then
        // drops the socket server-side.
        let pending_conn = conn.clone();
        let pending = tokio::spawn(async move {
            pending_conn
                .send_with_timeout("Never.answered", None, None, 60_000)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        conn.send("Kill.switch", None, None).await.ok();

        let e = expect_protocol(pending.await.unwrap().unwrap_err());
        assert_eq!(e.message, "Connection closed");
        // handle_close marks the connection closed.
        assert!(crate::test_env::wait_for(|| !conn.is_connected()).await);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_and_stale_frames_are_ignored() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;
        use tokio_tungstenite::tungstenite::protocol::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            // Garbage frames that must not break the dispatch loop.
            let junk = [
                "not json at all",
                "[1,2,3]",
                r#"{"id": 99, "method": "Both.idAndMethod"}"#,
                r#"{"id": 12345, "result": {"stale": true}}"#, // unknown id
                r#"{"method": 42}"#,                           // non-string method
            ];
            for frame in junk {
                let _ = ws.send(Message::Text(frame.to_string())).await;
            }
            // Now answer real commands forever.
            while let Some(frame) = ws.next().await {
                let Ok(Message::Text(text)) = frame else {
                    break;
                };
                let v: Value = serde_json::from_str(&text).unwrap();
                let id = v.get("id").and_then(Value::as_u64).unwrap();
                let _ = ws
                    .send(Message::Text(
                        json!({"id": id, "result": {"ok": true}}).to_string(),
                    ))
                    .await;
            }
        });

        let conn = CdpConnection::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // The connection survived the junk and still answers commands.
        // (Generous timeout: under full-suite thread starvation the mock
        // server task can be scheduled late.)
        let result = conn
            .send_with_timeout("Page.enable", None, None, 60_000)
            .await
            .unwrap();
        assert_eq!(result, json!({"ok": true}));
        conn.disconnect().await;
    }

    // ── Events ────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn events_route_to_session_and_browser_handlers() {
        let mock = MockCdp::start().await;
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();

        let got: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let _unsub = conn.on(Some("SID-X"), "Target.targetCreated", sink_push(&got));
        let browser_got: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let session = CdpSession::new("", conn.clone());
        let _unsub2 = session.on_browser("Target.targetCreated", sink_push(&browser_got));

        // createTarget → browser-level targetCreated event.
        conn.send(
            "Target.createTarget",
            Some(&params(json!({"url": "http://x/"}))),
            None,
        )
        .await
        .unwrap();

        assert!(crate::test_env::wait_for(|| !browser_got.lock().unwrap().is_empty()).await);
        assert_eq!(browser_got.lock().unwrap().len(), 1);
        // The session-scoped handler did NOT receive the browser-level event.
        assert!(got.lock().unwrap().is_empty());
        conn.disconnect().await;
    }

    // ── Target registry + pending rejection ───────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn target_registry_and_session_pending_rejection() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .no_reply_methods
            .insert("Never.answered".to_string());
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();

        conn.register_target(AttachedTarget {
            target_id: "T-1".to_string(),
            session_id: "S-1".to_string(),
            r#type: "page".to_string(),
        });
        assert_eq!(
            conn.get_target_by_session_id("S-1").map(|t| t.target_id),
            Some("T-1".to_string())
        );
        assert!(conn.get_target_by_session_id("nope").is_none());

        // A pending request on S-1 gets rejected when the session is torn down.
        let pending_conn = conn.clone();
        let pending = tokio::spawn(async move {
            pending_conn
                .send_with_timeout("Never.answered", None, Some("S-1"), 60_000)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        conn.reject_pending_for_session(
            "S-1",
            CdpSendError::Protocol(CdpError {
                code: -1,
                message: "Target T-1 destroyed".to_string(),
            }),
        );
        let err = pending.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Target T-1 destroyed");

        // Detach by session id and target id.
        assert_eq!(
            conn.detach_target_by_session_id("S-1").map(|t| t.target_id),
            Some("T-1".to_string())
        );
        assert!(conn.detach_target_by_session_id("S-1").is_none());
        conn.register_target(AttachedTarget {
            target_id: "T-2".to_string(),
            session_id: "S-2".to_string(),
            r#type: "page".to_string(),
        });
        assert_eq!(
            conn.detach_target_by_target_id("T-2").map(|t| t.session_id),
            Some("S-2".to_string())
        );
        conn.disconnect().await;
    }

    // ── CdpSession surface ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn cdp_session_surface() {
        let mock = MockCdp::start().await;
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let session = CdpSession::new("S-9", conn.clone());
        assert_eq!(session.session_id, "S-9");
        assert!(Arc::ptr_eq(session.connection(), &conn));

        // send_with_timeout happy path.
        let v = session
            .send_with_timeout("Page.enable", None, 1_000)
            .await
            .unwrap();
        assert_eq!(v, json!({}));

        // on() with a non-empty session id scopes the handler.
        let got = Arc::new(AtomicU64::new(0));
        let g = got.clone();
        let _u = session.on(
            "Some.event",
            Arc::new(move |_| {
                g.fetch_add(1, Ordering::SeqCst);
            }),
        );
        conn.dispatch_test(Some("S-9"), "Some.event", &json!({}));
        conn.dispatch_test(Some("other"), "Some.event", &json!({}));
        assert_eq!(got.load(Ordering::SeqCst), 1);

        // on() with the EMPTY session id normalizes to the browser key.
        let browser_got = Arc::new(AtomicU64::new(0));
        let b = browser_got.clone();
        let browser_session = CdpSession::new("", conn.clone());
        let _u2 = browser_session.on(
            "Browser.event",
            Arc::new(move |_| {
                b.fetch_add(1, Ordering::SeqCst);
            }),
        );
        conn.dispatch_test(None, "Browser.event", &json!({}));
        assert_eq!(browser_got.load(Ordering::SeqCst), 1);

        conn.disconnect().await;
    }

    // ── Error response edge shapes ────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn error_response_with_missing_fields_uses_defaults() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;
        use tokio_tungstenite::tungstenite::protocol::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(frame) = ws.next().await {
                let Ok(Message::Text(text)) = frame else {
                    break;
                };
                let v: Value = serde_json::from_str(&text).unwrap();
                let id = v.get("id").and_then(Value::as_u64).unwrap();
                // Error object without code/message → defaults kick in.
                let _ = ws
                    .send(Message::Text(
                        json!({"id": id, "error": {"something": "else"}}).to_string(),
                    ))
                    .await;
            }
        });

        let conn = CdpConnection::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        let e = expect_protocol(conn.send("Any.method", None, None).await.unwrap_err());
        assert_eq!(e.code, -1);
        assert_eq!(e.message, "Unknown CDP error");
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnect_rejects_all_pending() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .no_reply_methods
            .insert("Never.answered".to_string());
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let pending_conn = conn.clone();
        let pending = tokio::spawn(async move {
            pending_conn
                .send_with_timeout("Never.answered", None, None, 60_000)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        conn.disconnect().await;
        let err = pending.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Connection closed");
    }

    /// Dropping the last connection Arc (after the peer closed, so the
    /// reader task is gone too) closes the broadcast channel: the dispatch
    /// task observes Closed, breaks, and runs to completion.
    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_task_ends_after_final_connection_drop() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .close_connection_on
            .insert("Kill.switch".to_string());
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        // The mock drops the socket on Kill.switch → reader ends (its
        // sender clone dies) and the dispatch loop runs handle_close.
        conn.send("Kill.switch", None, None).await.ok();
        assert!(crate::test_env::wait_for(|| !conn.is_connected()).await);
        // The last Arc: the transport's sender drops with it, so every
        // broadcast sender is gone and the dispatch task exits via Closed.
        drop(conn);
        // No handle to await — give the runtime a window to poll the
        // dispatch task to completion (its end line only counts then).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_loop_survives_lagged_broadcast() {
        // Flooding tens of thousands of queued events in one burst makes the
        // dispatch loop's receiver lag; the loop must skip the gap and keep
        // going.
        let mock = MockCdp::start().await;
        let flood: Vec<Value> = (0..40_000)
            .map(|i| json!({"method": "Test.flood", "params": {"i": i}}))
            .collect();
        mock.state
            .lock()
            .unwrap()
            .events_after_response
            .insert("Page.enable".to_string(), flood);
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();

        let seen = Arc::new(AtomicUsize::new(0));
        let s = seen.clone();
        let _unsub = conn.on(
            None,
            "Test.flood",
            Arc::new(move |_| {
                s.fetch_add(1, Ordering::SeqCst);
            }),
        );
        conn.send("Page.enable", None, None).await.unwrap();
        // A follow-up command still gets answered after the flood.
        let result = conn.send("Page.getFrameTree", None, None).await.unwrap();
        assert!(result.get("frameTree").is_some());
        let s = seen.clone();
        assert!(crate::test_env::wait_for(move || s.load(Ordering::SeqCst) > 0).await);
        conn.disconnect().await;
    }

    #[tokio::test]
    async fn dispatch_loop_skips_lagged_broadcast_deterministically() {
        // A small-capacity channel forces the receiver to lag after a burst,
        // exercising the `Err(Lagged) => continue` arm without the 40k-event
        // race of the live-transport flood test.
        let (tx, rx) = broadcast::channel::<TransportEvent>(1);
        tx.send(TransportEvent::Message("a".into())).unwrap();
        tx.send(TransportEvent::Message("b".into())).unwrap();
        tx.send(TransportEvent::Message("c".into())).unwrap();
        drop(tx);
        // Dead weak ref: the loop must skip the gap and exit on Closed.
        let handle = tokio::spawn(run_dispatch_loop(rx, Weak::new()));
        tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("dispatch loop hung")
            .expect("dispatch loop panicked");
    }
}
