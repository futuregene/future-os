//! Execution context tracker — port of
//! `cli/src/browser/chromium/chromium-execution-context.ts`.
//!
//! Per-CdpSession management of Runtime.executionContextCreated/Destroyed/
//! Cleared events. One tracker per session; dispose() removes listeners.

use super::cdp_connection::CdpSession;
use crate::browser::backend::Deadline;
use crate::browser::chromium::cdp_event_router::{CdpEventHandler, Unsubscribe};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// `ExecutionContextInfo`.
#[derive(Debug, Clone)]
pub struct ExecutionContextInfo {
    pub context_id: i64,
    pub frame_id: String,
    pub is_default: bool,
    pub name: String,
}

/// `ExecutionContextTracker`.
pub struct ExecutionContextTracker {
    contexts: Arc<Mutex<HashMap<i64, ExecutionContextInfo>>>,
    _unsubs: Vec<Unsubscribe>,
}

impl ExecutionContextTracker {
    pub fn new(session: &CdpSession) -> Self {
        let mut tracker = ExecutionContextTracker {
            contexts: Arc::new(Mutex::new(HashMap::new())),
            _unsubs: Vec::new(),
        };
        tracker.subscribe(session);
        tracker
    }

    fn subscribe(&mut self, session: &CdpSession) {
        {
            let contexts = self.contexts.clone();
            let h: CdpEventHandler = std::sync::Arc::new(move |params: &Value| {
                let context = params.get("context").and_then(Value::as_object);
                if let Some(ctx) = context {
                    let id = ctx.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let aux = ctx.get("auxData").and_then(Value::as_object);
                    let frame_id = aux
                        .and_then(|a| a.get("frameId"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let is_default = aux
                        .and_then(|a| a.get("isDefault"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let name = ctx
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Ok(mut map) = contexts.lock() {
                        map.insert(
                            id,
                            ExecutionContextInfo {
                                context_id: id,
                                frame_id,
                                is_default,
                                name,
                            },
                        );
                    }
                }
            });
            self._unsubs
                .push(session.on("Runtime.executionContextCreated", h));
        }
        {
            let contexts = self.contexts.clone();
            let h: CdpEventHandler = std::sync::Arc::new(move |params: &Value| {
                let id = params
                    .get("executionContextId")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                if let Ok(mut map) = contexts.lock() {
                    map.remove(&id);
                }
            });
            self._unsubs
                .push(session.on("Runtime.executionContextDestroyed", h));
        }
        {
            let contexts = self.contexts.clone();
            let h: CdpEventHandler = std::sync::Arc::new(move |_params: &Value| {
                if let Ok(mut map) = contexts.lock() {
                    map.clear();
                }
            });
            self._unsubs
                .push(session.on("Runtime.executionContextsCleared", h));
        }
    }

    /// `getMainWorldContextId(frameId, deadline)` — exact frameId match, then
    /// any default-world context, polling every 50 ms until the deadline.
    pub async fn get_main_world_context_id(
        &self,
        frame_id: &str,
        deadline: &Deadline,
    ) -> Result<i64, String> {
        while !deadline.expired() {
            {
                let map = self.contexts.lock().unwrap();
                for ctx in map.values() {
                    if ctx.frame_id == frame_id && ctx.is_default {
                        return Ok(ctx.context_id);
                    }
                }
                for ctx in map.values() {
                    if ctx.is_default {
                        return Ok(ctx.context_id);
                    }
                }
            }
            crate::utils::time::sleep(50).await;
        }
        Err("No execution context found within timeout".to_string())
    }

    /// `dispose()` — remove all event listeners.
    pub fn dispose(&mut self) {
        for unsub in self._unsubs.drain(..) {
            unsub.unsubscribe();
        }
        self.contexts.lock().unwrap().clear();
    }

    /// Test-only: number of live event subscriptions.
    #[cfg(test)]
    fn subscription_count(&self) -> usize {
        self._unsubs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::chromium::cdp_connection::CdpConnection;
    use futures_util::SinkExt;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::protocol::Message;

    /// Spawn a WebSocket server that sends `script` (CDP events/responses)
    /// shortly after accepting, then holds the connection open.
    async fn scripted_server(script: Vec<Value>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            // Give the client's dispatch loop time to subscribe.
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            for msg in script {
                let _ = ws.send(Message::Text(msg.to_string())).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });
        (format!("ws://{addr}"), handle)
    }

    async fn connect(
        script: Vec<Value>,
    ) -> (Arc<CdpConnection>, CdpSession, tokio::task::JoinHandle<()>) {
        let (url, server) = scripted_server(script).await;
        let conn = CdpConnection::connect(&url, 5000).await.expect("connect");
        let session = CdpSession::new("sess-1", conn.clone());
        (conn, session, server)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn created_event_edge_shapes_use_defaults() {
        let (conn, _session, server) = connect(vec![]).await;
        let tracker = ExecutionContextTracker::new(&_session);

        // No "context" object → ignored.
        conn.dispatch_test(None, "Runtime.executionContextCreated", &json!({}));
        // Context without id / auxData / name → id 0, "" frame, default.
        conn.dispatch_test(
            None,
            "Runtime.executionContextCreated",
            &json!({"context": {}}),
        );
        // auxData present but partial → is_default honored, frame "".
        conn.dispatch_test(
            None,
            "Runtime.executionContextCreated",
            &json!({"context": {"id": 7, "auxData": {"isDefault": false}}}),
        );
        // Destroyed without id → removes id 0 (the first default entry).
        conn.dispatch_test(None, "Runtime.executionContextDestroyed", &json!({}));
        // Cleared empties everything.
        conn.dispatch_test(None, "Runtime.executionContextsCleared", &json!({}));

        // A subsequent default-world lookup must NOT find the cleared ones.
        let result = tracker
            .get_main_world_context_id("any", &Deadline::new(120))
            .await;
        assert!(result.is_err());
        drop(server);
    }

    #[test]
    fn execution_context_info_shape() {
        let info = ExecutionContextInfo {
            context_id: 3,
            frame_id: "f".to_string(),
            is_default: true,
            name: "n".to_string(),
        };
        let cloned = info.clone();
        assert_eq!(cloned.context_id, 3);
        let _ = format!("{info:?}");
    }

    fn created(id: i64, frame_id: &str, is_default: bool) -> Value {
        json!({
            "sessionId": "sess-1",
            "method": "Runtime.executionContextCreated",
            "params": {
                "context": {"id": id, "auxData": {"frameId": frame_id, "isDefault": is_default}, "name": "main"}
            }
        })
    }

    fn destroyed(id: i64) -> Value {
        json!({
            "sessionId": "sess-1",
            "method": "Runtime.executionContextDestroyed",
            "params": {"executionContextId": id}
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tracks_created_contexts() {
        let (_conn, session, _server) = connect(vec![created(1, "f1", true)]).await;
        let mut tracker = ExecutionContextTracker::new(&session);
        let id = tracker
            .get_main_world_context_id("f1", &Deadline::new(1000))
            .await
            .unwrap();
        assert_eq!(id, 1);
        tracker.dispose();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn falls_back_to_any_default_context() {
        let (_conn, session, _server) = connect(vec![created(42, "other_frame", true)]).await;
        let mut tracker = ExecutionContextTracker::new(&session);
        let id = tracker
            .get_main_world_context_id("unknown_frame", &Deadline::new(1000))
            .await
            .unwrap();
        assert_eq!(id, 42);
        tracker.dispose();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn destroyed_contexts_are_removed() {
        let (_conn, session, _server) = connect(vec![created(1, "f1", true), destroyed(1)]).await;
        let mut tracker = ExecutionContextTracker::new(&session);
        let result = tracker
            .get_main_world_context_id("f1", &Deadline::new(150))
            .await;
        assert!(result.is_err());
        tracker.dispose();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleared_event_removes_all_contexts() {
        let (_conn, session, _server) = connect(vec![
            created(1, "f1", true),
            created(2, "f2", true),
            json!({"sessionId": "sess-1", "method": "Runtime.executionContextsCleared", "params": {}}),
        ])
        .await;
        let mut tracker = ExecutionContextTracker::new(&session);
        let result = tracker
            .get_main_world_context_id("f1", &Deadline::new(150))
            .await;
        assert!(result.is_err());
        tracker.dispose();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_default_contexts_are_skipped_for_main_world() {
        let (_conn, session, _server) = connect(vec![created(1, "f1", false)]).await;
        let mut tracker = ExecutionContextTracker::new(&session);
        let result = tracker
            .get_main_world_context_id("f1", &Deadline::new(150))
            .await;
        assert!(result.is_err());
        tracker.dispose();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispose_removes_listeners() {
        let (_conn, session, _server) = connect(vec![]).await;
        let mut tracker = ExecutionContextTracker::new(&session);
        assert!(tracker.subscription_count() > 0);
        tracker.dispose();
        assert_eq!(tracker.subscription_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn context_appears_after_creation_event() {
        // The server sends the event 80 ms after connect — the get call must
        // poll until it lands.
        let (_conn, session, _server) = connect(vec![created(7, "f1", true)]).await;
        let mut tracker = ExecutionContextTracker::new(&session);
        let id = tracker
            .get_main_world_context_id("f1", &Deadline::new(3000))
            .await
            .unwrap();
        assert_eq!(id, 7);
        tracker.dispose();
    }
}
