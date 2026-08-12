//! Test-only mock CDP browser: a WebSocket server speaking enough of the
//! Chrome DevTools Protocol to drive the whole chromium stack (connection,
//! page manager, session, navigation, screenshots) plus an HTTP responder
//! for `/json/version` endpoint resolution.
//!
//! The mock is stateful and scriptable: tests mutate [`MockCdpState`] (via
//! the shared `state` handle) to inject failures, navigation events, and
//! `Runtime.evaluate` return values.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message;

/// One mock page target.
#[derive(Debug, Clone)]
pub struct MockTarget {
    pub target_id: String,
    pub url: String,
    pub title: String,
    /// CDP target type ("page", "worker", ...). The page manager only
    /// attaches to "page" targets.
    pub kind: String,
}

/// Test hook: expression → `Some(value)` to override the default
/// `Runtime.evaluate` responder.
pub type EvalOverride = Arc<dyn Fn(&str) -> Option<Value> + Send + Sync>;

/// Shared mutable mock state (locked briefly per inbound command).
pub struct MockCdpState {
    pub targets: Vec<MockTarget>,
    /// sessionId → targetId.
    pub sessions: HashMap<String, String>,
    /// Methods answered with a CDP error `{code: -32000, message: "mock
    /// failure"}` instead of a result.
    pub fail_methods: HashSet<String>,
    /// Fail only the Nth (0-based) invocation of a method — multi-call
    /// sequences like dispatchEnter (rawKeyDown/char/keyUp share one CDP
    /// method name).
    pub fail_on_call: HashSet<(String, usize)>,
    /// Methods the mock never answers (client-side timeout tests).
    pub no_reply_methods: HashSet<String>,
    /// Methods that make the mock drop the WebSocket WITHOUT answering
    /// (server-initiated close tests).
    pub close_connection_on: HashSet<String>,
    /// `Page.navigate` → `{errorText: ...}` response.
    pub navigate_error_text: Option<String>,
    /// `Page.navigate` → same-document response (no loaderId).
    pub navigate_same_document: bool,
    /// Do not emit the DOMContentLoaded lifecycle event after navigate.
    pub suppress_loaded_event: bool,
    /// Do not emit Target.targetCreated after Target.createTarget
    /// (page-manager discovery-timeout tests).
    pub suppress_target_created: bool,
    /// Page.getLayoutMetrics result payload.
    pub layout_metrics: Value,
    /// Emit a fresh-loader DOMContentLoaded lifecycle event after every
    /// Input.dispatch* command (simulates action-triggered navigation).
    pub navigate_on_input: bool,
    /// Value returned for the `__futureConsoleLogs` read expression.
    pub console_logs: Value,
    /// `items` returned for the snapshot function expression.
    pub snapshot_items: Vec<Value>,
    /// `title` returned for the snapshot function / `document.title`.
    pub snapshot_title: String,
    /// Value returned for the element-check script (exists/visible/...).
    pub element_check: Value,
    /// Value returned for the click-metadata script (href/hasSubmitter).
    pub click_meta: Value,
    /// Value returned for the click-state read script.
    pub click_state: Value,
    /// Base64 payload for Page.captureScreenshot.
    pub screenshot_b64: String,
    /// Extra events emitted right after the response to a given method
    /// (e.g. a bare Target.targetCreated after setDiscoverTargets).
    pub events_after_response: HashMap<String, Vec<Value>>,
    /// Runtime.evaluate expressions containing this substring get a CDP
    /// error response (per-expression failure injection).
    pub eval_error_on_substring: Option<String>,
    /// Runtime.evaluate expressions containing this substring make the mock
    /// DROP the WebSocket without answering (mid-session death tests).
    pub close_connection_on_eval_substring: Option<String>,
    /// Per-test Runtime.evaluate override (checked FIRST).
    pub eval_override: Option<EvalOverride>,
    /// Every command received: (method, sessionId, params).
    pub commands: Vec<(String, Option<String>, Value)>,
    /// Per-method invocation counter backing [`Self::fail_on_call`].
    call_counts: HashMap<String, usize>,
    next_target: u64,
    next_session: u64,
    next_loader: u64,
    next_preload: u64,
}

impl Default for MockCdpState {
    fn default() -> Self {
        MockCdpState {
            targets: Vec::new(),
            sessions: HashMap::new(),
            fail_methods: HashSet::new(),
            fail_on_call: HashSet::new(),
            no_reply_methods: HashSet::new(),
            close_connection_on: HashSet::new(),
            navigate_error_text: None,
            navigate_same_document: false,
            suppress_loaded_event: false,
            suppress_target_created: false,
            layout_metrics: json!({"cssContentSize": {"x": 0, "y": 0, "width": 800, "height": 600}}),
            navigate_on_input: false,
            console_logs: Value::Array(Vec::new()),
            snapshot_items: Vec::new(),
            snapshot_title: "Mock Title".to_string(),
            element_check: json!({
                "exists": true, "connected": true, "visible": true,
                "disabled": false,
                "box": {"x": 10, "y": 20, "width": 100, "height": 30},
                "obscured": false,
            }),
            click_meta: json!({"href": null, "hasSubmitter": false}),
            click_state: json!({"defaultPrevented": false, "submitSeen": false}),
            screenshot_b64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                b"\x89PNG-mock",
            ),
            events_after_response: HashMap::new(),
            eval_error_on_substring: None,
            close_connection_on_eval_substring: None,
            eval_override: None,
            commands: Vec::new(),
            call_counts: HashMap::new(),
            next_target: 0,
            next_session: 0,
            next_loader: 1,
            next_preload: 0,
        }
    }
}

/// A running mock browser.
pub struct MockCdp {
    /// Base HTTP URL serving `/json/version` (via `test_server::spawn_http`).
    pub http_url: String,
    /// `ws://` URL of the CDP WebSocket endpoint.
    pub ws_url: String,
    pub state: Arc<Mutex<MockCdpState>>,
    shutdown: Arc<tokio::sync::Notify>,
    accept: tokio::task::JoinHandle<()>,
}

impl MockCdp {
    /// Start the mock with one initial page target ("about:blank").
    pub async fn start() -> Self {
        Self::start_with(vec![target("T-1", "about:blank", "")], "Chrome/126.0.0.0").await
    }

    /// Start with explicit initial targets and a Browser identity string.
    pub async fn start_with(initial_targets: Vec<MockTarget>, browser: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
        let ws_addr = listener.local_addr().expect("local_addr");
        let ws_url = format!("ws://{ws_addr}/devtools/browser/mock");

        let seeded = initial_targets.len() as u64;
        let state = Arc::new(Mutex::new(MockCdpState {
            targets: initial_targets,
            next_target: seeded,
            ..MockCdpState::default()
        }));

        let server_state = state.clone();
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let accept_shutdown = shutdown.clone();
        let accept = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = accept_shutdown.notified() => break,
                    accepted = listener.accept() => {
                        // Invariant: accept on a live mock listener only
                        // fails on OS-level resource exhaustion.
                        let (stream, _) = accepted.expect("mock ws listener accept");
                        let conn_state = server_state.clone();
                        tokio::spawn(async move {
                            let Ok(mut ws) = accept_async(stream).await else {
                                return;
                            };
                            while let Some(frame) = ws.next().await {
                                let Ok(Message::Text(text)) = frame else {
                                    break;
                                };
                                let Some(replies) = handle_cdp_message(&conn_state, &text) else {
                                    // close_connection_on triggered — drop the socket.
                                    return;
                                };
                                for reply in replies {
                                    if ws.send(Message::Text(reply.to_string())).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        });
                    }
                }
            }
        });

        // HTTP side: /json/version pointing at the WS endpoint.
        let body = json!({
            "Browser": browser,
            "User-Agent": format!("Mozilla/5.0 (Macintosh) {browser} Safari/537.36"),
            "webSocketDebuggerUrl": ws_url,
        })
        .to_string();
        let http_url = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/json/version",
            200,
            &body,
        )])
        .await;

        MockCdp {
            http_url,
            ws_url,
            state,
            shutdown,
            accept,
        }
    }

    /// Stop accepting new connections and wait for the accept task to run
    /// to completion (its end lines only count when the task finishes
    /// inside the test). In-flight connections are unaffected.
    pub async fn shutdown(self) {
        self.shutdown.notify_one();
        self.accept.await.expect("accept task completes");
    }

    /// Recorded commands of the given method.
    pub fn commands_of(&self, method: &str) -> Vec<Value> {
        self.state
            .lock()
            .expect("state")
            .commands
            .iter()
            .filter(|(m, _, _)| m == method)
            .map(|(_, _, p)| p.clone())
            .collect()
    }
}

/// Handle one inbound text frame; returns the frames to send back (the
/// command response first, then any events to emit). `None` asks the
/// connection task to drop the socket (close_connection_on).
fn handle_cdp_message(state_arc: &Arc<Mutex<MockCdpState>>, text: &str) -> Option<Vec<Value>> {
    let message: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Some(Vec::new()),
    };
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        return Some(Vec::new());
    };
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let session_id = message
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let mut events: Vec<Value> = Vec::new();
    let result: Value = {
        let mut state = state_arc.lock().expect("state");
        state
            .commands
            .push((method.clone(), session_id.clone(), params.clone()));
        if state.close_connection_on.contains(&method) {
            drop(state);
            return None;
        }
        if method == "Runtime.evaluate" {
            let expression = params
                .get("expression")
                .and_then(Value::as_str)
                .unwrap_or("");
            if state
                .close_connection_on_eval_substring
                .as_deref()
                .is_some_and(|sub| expression.contains(sub))
            {
                drop(state);
                return None;
            }
        }
        if state.no_reply_methods.contains(&method) {
            return Some(Vec::new());
        }
        let call_index = {
            let count = state.call_counts.entry(method.clone()).or_insert(0);
            let index = *count;
            *count += 1;
            index
        };
        if state.fail_methods.contains(&method)
            || state.fail_on_call.contains(&(method.clone(), call_index))
        {
            return Some(vec![json!({
                "id": id,
                "error": {"code": -32000, "message": "mock failure"},
            })]);
        }
        let result = dispatch_method(
            &mut state,
            &method,
            &params,
            session_id.as_deref(),
            &mut events,
        );
        if let Some(extra) = state.events_after_response.get(&method) {
            events.extend(extra.iter().cloned());
        }
        result
    };

    // Per-expression eval failure marker → protocol error frame.
    if result.get("__eval_error__").is_some() {
        return Some(vec![json!({
            "id": id,
            "error": {"code": -32000, "message": "eval failure"},
        })]);
    }

    let mut out = vec![json!({"id": id, "result": result})];
    out.extend(events);
    Some(out)
}

/// The target a session is attached to (first target for browser-level).
fn target_for<'a>(state: &'a MockCdpState, session_id: Option<&str>) -> Option<&'a MockTarget> {
    let target_id = session_id.and_then(|sid| state.sessions.get(sid));
    match target_id {
        Some(tid) => state.targets.iter().find(|t| &t.target_id == tid),
        None => state.targets.first(),
    }
}

fn frame_id_for(state: &MockCdpState, session_id: Option<&str>) -> String {
    target_for(state, session_id)
        .map(|t| format!("frame-{}", t.target_id))
        .unwrap_or_else(|| "frame-main".to_string())
}

fn dispatch_method(
    state: &mut MockCdpState,
    method: &str,
    params: &Value,
    session_id: Option<&str>,
    events: &mut Vec<Value>,
) -> Value {
    match method {
        "Target.setDiscoverTargets" | "Target.setAutoAttach" => json!({}),

        "Target.getTargets" => {
            let infos: Vec<Value> = state
                .targets
                .iter()
                .map(|t| {
                    json!({
                        "targetId": t.target_id,
                        "type": t.kind,
                        "url": t.url,
                        "title": t.title,
                    })
                })
                .collect();
            json!({"targetInfos": infos})
        }

        "Target.attachToTarget" => {
            let target_id = params
                .get("targetId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            state.next_session += 1;
            let sid = format!("SID-{}", state.next_session);
            state.sessions.insert(sid.clone(), target_id);
            json!({"sessionId": sid})
        }

        "Target.createTarget" => {
            let url = params
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank")
                .to_string();
            state.next_target += 1;
            let target_id = format!("T-{}", state.next_target);
            state.targets.push(MockTarget {
                target_id: target_id.clone(),
                url: url.clone(),
                title: String::new(),
                kind: "page".to_string(),
            });
            if !state.suppress_target_created {
                events.push(json!({
                    "method": "Target.targetCreated",
                    "params": {"targetInfo": {
                        "targetId": target_id, "type": "page",
                        "url": url, "title": "",
                    }},
                }));
            }
            json!({"targetId": target_id})
        }

        "Target.closeTarget" => {
            let target_id = params
                .get("targetId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            state.targets.retain(|t| t.target_id != target_id);
            events.push(json!({
                "method": "Target.targetDestroyed",
                "params": {"targetId": target_id},
            }));
            json!({})
        }

        "Target.activateTarget" => json!({}),

        "Page.enable" | "Runtime.enable" | "Page.setLifecycleEventsEnabled" => json!({}),

        "Page.getFrameTree" => {
            let frame_id = frame_id_for(state, session_id);
            let url = target_for(state, session_id)
                .map(|t| t.url.clone())
                .unwrap_or_default();
            json!({"frameTree": {"frame": {
                "id": frame_id,
                "loaderId": "loader-1",
                "url": url,
            }}})
        }

        "Page.navigate" => {
            let url = params
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let frame_id = frame_id_for(state, session_id);
            if let Some(error_text) = &state.navigate_error_text {
                return json!({"errorText": error_text});
            }
            if state.navigate_same_document {
                return json!({"frameId": frame_id});
            }
            state.next_loader += 1;
            let loader_id = format!("loader-{}", state.next_loader);
            if let Some(target) = target_for_mut(state, session_id) {
                target.url = url;
            }
            if !state.suppress_loaded_event {
                let mut event = json!({
                    "method": "Page.lifecycleEvent",
                    "params": {"frameId": frame_id, "loaderId": loader_id, "name": "DOMContentLoaded"},
                });
                if let Some(sid) = session_id {
                    event["sessionId"] = Value::String(sid.to_string());
                }
                events.push(event);
            }
            json!({"frameId": frame_id, "loaderId": loader_id})
        }

        "Runtime.evaluate" => {
            let expression = params
                .get("expression")
                .and_then(Value::as_str)
                .unwrap_or("");
            if state
                .eval_error_on_substring
                .as_deref()
                .is_some_and(|sub| expression.contains(sub))
            {
                // Protocol-level error for this expression only.
                return json!({"__eval_error__": true});
            }
            let value = evaluate_response(state, expression, session_id);
            json!({"result": {"type": "object", "value": value}})
        }

        "Page.addScriptToEvaluateOnNewDocument" => {
            state.next_preload += 1;
            json!({"identifier": format!("preload-{}", state.next_preload)})
        }

        "Page.removeScriptToEvaluateOnNewDocument" => json!({}),

        "Page.captureScreenshot" => json!({"data": state.screenshot_b64}),

        "Page.getLayoutMetrics" => state.layout_metrics.clone(),

        "Input.dispatchMouseEvent" | "Input.dispatchKeyEvent" | "Input.insertText" => {
            if state.navigate_on_input {
                state.next_loader += 1;
                let loader_id = format!("loader-{}", state.next_loader);
                let frame_id = frame_id_for(state, session_id);
                let mut event = json!({
                    "method": "Page.lifecycleEvent",
                    "params": {"frameId": frame_id, "loaderId": loader_id, "name": "DOMContentLoaded"},
                });
                if let Some(sid) = session_id {
                    event["sessionId"] = Value::String(sid.to_string());
                }
                events.push(event);
            }
            json!({})
        }

        _ => json!({}),
    }
}

fn target_for_mut<'a>(
    state: &'a mut MockCdpState,
    session_id: Option<&str>,
) -> Option<&'a mut MockTarget> {
    let target_id = session_id.and_then(|sid| state.sessions.get(sid)).cloned();
    match target_id {
        Some(tid) => state.targets.iter_mut().find(|t| t.target_id == tid),
        None => state.targets.first_mut(),
    }
}

/// The default `Runtime.evaluate` responder — pattern-matches the injected
/// scripts by content marker.
fn evaluate_response(state: &MockCdpState, expression: &str, session_id: Option<&str>) -> Value {
    if let Some(override_fn) = &state.eval_override {
        if let Some(value) = override_fn(expression) {
            return value;
        }
    }
    if expression.contains("__futureClickState") {
        if expression.contains("addEventListener") {
            return state.click_meta.clone();
        }
        return state.click_state.clone();
    }
    if expression.contains("getComputedStyle") {
        return state.element_check.clone();
    }
    if expression.contains("scrollIntoView") {
        return json!({"x": 10, "y": 20, "width": 100, "height": 30});
    }
    if expression.contains("__futureConsoleHookInstalled") {
        return Value::Null;
    }
    if expression.contains("__futureConsoleLogs") {
        return state.console_logs.clone();
    }
    if expression.contains("escapeCss") {
        return json!({
            "title": state.snapshot_title,
            "url": target_for(state, session_id)
                .map(|t| t.url.clone())
                .unwrap_or_default(),
            "items": state.snapshot_items,
        });
    }
    if expression.contains("document.title") {
        return Value::String(state.snapshot_title.clone());
    }
    if expression.contains("location.href") {
        return Value::String(
            target_for(state, session_id)
                .map(|t| t.url.clone())
                .unwrap_or_else(|| "about:blank".to_string()),
        );
    }
    Value::Null
}

/// A `MockTarget` constructor for tests.
pub fn target(id: &str, url: &str, title: &str) -> MockTarget {
    MockTarget {
        target_id: id.to_string(),
        url: url.to_string(),
        title: title.to_string(),
        kind: "page".to_string(),
    }
}

/// A non-page target (worker/service_worker) for discovery-filter tests.
pub fn target_kind(id: &str, url: &str, title: &str, kind: &str) -> MockTarget {
    MockTarget {
        target_id: id.to_string(),
        url: url.to_string(),
        title: title.to_string(),
        kind: kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `shutdown()` ends the accept loop: the accept task observes the
    /// notify, breaks, and runs to completion (its end lines count).
    #[tokio::test(flavor = "multi_thread")]
    async fn accept_task_completes_on_shutdown() {
        let mock = MockCdp::start().await;
        // A connection still works right up to the shutdown.
        let conn =
            crate::browser::chromium::cdp_connection::CdpConnection::connect(&mock.ws_url, 5_000)
                .await
                .unwrap();
        conn.disconnect().await;
        mock.shutdown().await;
    }

    #[test]
    fn handle_cdp_message_frame_edge_shapes() {
        let state = Arc::new(Mutex::new(MockCdpState::default()));

        // Malformed JSON → ignored.
        assert_eq!(handle_cdp_message(&state, "not json"), Some(Vec::new()));
        // Valid JSON without an id → ignored (event-shaped frame).
        assert_eq!(
            handle_cdp_message(&state, r#"{"method":"Page.enable"}"#),
            Some(Vec::new())
        );
        // Unknown method → empty result.
        let replies = handle_cdp_message(&state, r#"{"id":1,"method":"Bogus.method"}"#).unwrap();
        assert_eq!(replies[0]["result"], json!({}));
        // Unknown method WITH a session id (no mapping → first-target arm of
        // target_for with an empty target list → None).
        let replies =
            handle_cdp_message(&state, r#"{"id":2,"method":"Nope.x","sessionId":"ghost"}"#)
                .unwrap();
        assert_eq!(replies[0]["result"], json!({}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reply_send_failure_when_client_vanishes_mid_flood() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().events_after_response.insert(
            "Page.enable".to_string(),
            (0..5_000)
                .map(|i| json!({"method": "E.v", "params": {"i": i}}))
                .collect(),
        );
        // Raw WS client: send the command, then drop the socket instantly —
        // the reply flood errors mid-send and the task returns early.
        let (mut ws, _) = tokio_tungstenite::connect_async(&mock.ws_url)
            .await
            .unwrap();
        ws.send(Message::Text(
            r#"{"id":1,"method":"Page.enable"}"#.to_string(),
        ))
        .await
        .unwrap();
        drop(ws);
        // The mock stays usable for a fresh connection.
        let conn =
            crate::browser::chromium::cdp_connection::CdpConnection::connect(&mock.ws_url, 5_000)
                .await
                .unwrap();
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn snapshot_eval_block_and_handshake_failure() {
        let mock = MockCdp::start().await;
        // Drive the snapshot eval response through a real CDP evaluate.
        let conn =
            crate::browser::chromium::cdp_connection::CdpConnection::connect(&mock.ws_url, 5_000)
                .await
                .unwrap();
        let session = crate::browser::chromium::cdp_connection::CdpSession::new("", conn.clone());
        let out = session
            .send(
                "Runtime.evaluate",
                Some(
                    &json!({"expression": "function(){ var escapeCss = 1; }"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(out["result"]["value"]["title"], json!("Mock Title"));
        conn.disconnect().await;

        // A raw TCP client that sends garbage → the WS handshake fails and
        // the connection task returns early.
        let host_port = mock.ws_url[5..].split('/').next().unwrap().to_string();
        let mut socket = tokio::net::TcpStream::connect(host_port).await.unwrap();
        use tokio::io::AsyncWriteExt;
        socket.write_all(b"garbage\r\n\r\n").await.unwrap();
        drop(socket);
        // The mock survives and answers a fresh connection.
        let conn2 =
            crate::browser::chromium::cdp_connection::CdpConnection::connect(&mock.ws_url, 5_000)
                .await
                .unwrap();
        conn2.disconnect().await;
    }
}
