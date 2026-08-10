//! Navigation waiter for Chromium CDP — port of
//! `cli/src/browser/chromium/chromium-navigation.ts`.
//!
//! Explicit navigation (open): Frame.navigate → wait for loaderId.
//! Action-triggered (click/press): capture current loaderId, wait for change.

use super::cdp_connection::CdpSession;
use crate::browser::backend::Deadline;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// `NavigationResult`.
#[derive(Debug, Clone, Default)]
pub struct NavigationResult {
    pub did_navigate: bool,
    pub new_url: Option<String>,
    pub error_text: Option<String>,
    pub same_document: Option<bool>,
}

// ── Explicit navigation ─────────────────────────────────────────────

/// `waitForExplicitNavigation(session, url, deadline)`.
pub async fn wait_for_explicit_navigation(
    session: &CdpSession,
    url: &str,
    deadline: &Deadline,
) -> Result<NavigationResult, String> {
    // Subscribe before Page.navigate so a fast DOMContentLoaded event cannot
    // be lost between the command response and listener registration.
    let loaded: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let loaded_cb = loaded.clone();
    let handler: crate::browser::chromium::cdp_event_router::CdpEventHandler =
        std::sync::Arc::new(move |event: &Value| {
            let loader_id = event.get("loaderId").and_then(Value::as_str);
            let name = event.get("name").and_then(Value::as_str);
            if let Some(lid) = loader_id {
                if name == Some("DOMContentLoaded") {
                    if let Ok(mut set) = loaded_cb.lock() {
                        set.insert(lid.to_string());
                    }
                }
            }
        });
    let unsub = session.on("Page.lifecycleEvent", handler);

    let result = async {
        let response = session
            .send(
                "Page.navigate",
                Some(&serde_json::json!({"url": url}).as_object().unwrap().clone()),
            )
            .await
            .map_err(|e| e.to_string())?;

        if let Some(error_text) = response.get("errorText").and_then(Value::as_str) {
            return Ok(NavigationResult {
                did_navigate: false,
                error_text: Some(error_text.to_string()),
                ..Default::default()
            });
        }

        let loader_id = response.get("loaderId").and_then(Value::as_str);
        let Some(loader_id) = loader_id else {
            // Same-document navigation
            return Ok(NavigationResult {
                did_navigate: true,
                same_document: Some(true),
                ..Default::default()
            });
        };

        let wait_ms = deadline.remaining_ms().min(5_000);
        wait_until(
            || {
                loaded
                    .lock()
                    .map(|set| set.contains(loader_id))
                    .unwrap_or(false)
            },
            wait_ms,
        )
        .await;

        Ok(NavigationResult {
            did_navigate: true,
            ..Default::default()
        })
    }
    .await;

    drop(unsub);
    result
}

// ── Action-triggered navigation ─────────────────────────────────────

/// `ActionNavigationObserver` — observes loaderId changes after a user action.
pub struct ActionNavigationObserver {
    main_frame_id: String,
    current_loader_id: String,
    new_loader_id: Arc<Mutex<Option<String>>>,
    loaded: Arc<Mutex<std::collections::HashSet<String>>>,
    disposed: Arc<AtomicBool>,
    _unsub: Mutex<Option<crate::browser::chromium::cdp_event_router::Unsubscribe>>,
}

impl ActionNavigationObserver {
    pub fn new(main_frame_id: &str, current_loader_id: &str) -> Self {
        ActionNavigationObserver {
            main_frame_id: main_frame_id.to_string(),
            current_loader_id: current_loader_id.to_string(),
            new_loader_id: Arc::new(Mutex::new(None)),
            loaded: Arc::new(Mutex::new(std::collections::HashSet::new())),
            disposed: Arc::new(AtomicBool::new(false)),
            _unsub: Mutex::new(None),
        }
    }

    /// `arm(session)` — register the lifecycle listener.
    pub fn arm(&self, session: &CdpSession) {
        if self.disposed.load(Ordering::SeqCst) {
            return;
        }
        let main_frame_id = self.main_frame_id.clone();
        let current_loader_id = self.current_loader_id.clone();
        let new_loader_id = self.new_loader_id.clone();
        let loaded = self.loaded.clone();
        let handler: crate::browser::chromium::cdp_event_router::CdpEventHandler =
            std::sync::Arc::new(move |event: &Value| {
                let frame_id = event.get("frameId").and_then(Value::as_str);
                // Only track main frame navigations — ignore iframes.
                if frame_id != Some(main_frame_id.as_str()) {
                    return;
                }
                let loader_id = event.get("loaderId").and_then(Value::as_str);
                let name = event.get("name").and_then(Value::as_str);
                if let Some(lid) = loader_id {
                    if name == Some("DOMContentLoaded") {
                        if let Ok(mut set) = loaded.lock() {
                            set.insert(lid.to_string());
                        }
                    }
                    if lid != current_loader_id {
                        if let Ok(mut slot) = new_loader_id.lock() {
                            *slot = Some(lid.to_string());
                        }
                    }
                }
            });
        let unsub = session.on("Page.lifecycleEvent", handler);
        *self._unsub.lock().unwrap() = Some(unsub);
    }

    /// `wait(session, deadline)` — poll up to 500 ms for a new loaderId.
    pub async fn wait(&self, deadline: &Deadline) -> Result<NavigationResult, String> {
        if self.disposed.load(Ordering::SeqCst) {
            return Ok(NavigationResult {
                did_navigate: false,
                ..Default::default()
            });
        }

        // Phase 1 — wait for navigation to *start* (max 500 ms).
        let nav_start_ms = deadline.remaining_ms().min(500);
        let nav_start_at = crate::utils::time::now_millis();
        while crate::utils::time::now_millis().saturating_sub(nav_start_at) < nav_start_ms {
            let new_loader = self.new_loader_id.lock().unwrap().clone();
            if let Some(_new_loader) = new_loader {
                // Phase 2 — navigation started; wait for DOMContentLoaded on
                // the new loader (read the current value inside the predicate
                // because redirects may replace loaderId).
                let wait_ms = deadline.remaining_ms().min(5_000);
                let new_loader_id = self.new_loader_id.clone();
                let loaded = self.loaded.clone();
                wait_until(
                    move || {
                        let current = new_loader_id.lock().unwrap().clone();
                        match current {
                            Some(lid) => {
                                loaded.lock().map(|set| set.contains(&lid)).unwrap_or(false)
                            }
                            None => false,
                        }
                    },
                    wait_ms,
                )
                .await;
                return Ok(NavigationResult {
                    did_navigate: true,
                    ..Default::default()
                });
            }
            crate::utils::time::sleep(50).await;
        }

        // No navigation started — action was a non-navigating interaction.
        Ok(NavigationResult {
            did_navigate: false,
            ..Default::default()
        })
    }

    /// `dispose()`.
    pub fn dispose(&self) {
        self.disposed.store(true, Ordering::SeqCst);
        *self._unsub.lock().unwrap() = None;
    }
}

// ── Internal ────────────────────────────────────────────────────────

async fn wait_until<F: FnMut() -> bool>(mut predicate: F, timeout_ms: u64) {
    let end = crate::utils::time::now_millis() + timeout_ms;
    while !predicate() && crate::utils::time::now_millis() < end {
        let remaining = end.saturating_sub(crate::utils::time::now_millis());
        crate::utils::time::sleep(remaining.clamp(1, 50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::chromium::cdp_connection::CdpConnection;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::protocol::Message;

    /// Fake CDP session over a scripted WebSocket: `send` answers Page.navigate
    /// with a fixed loaderId and fires the queued lifecycle events.
    async fn scripted_nav_server(
        events_before_reply: Vec<Value>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            // Wait for the client's Page.navigate command, fire queued events
            // BEFORE replying (the TS test: DOMContentLoaded fired before
            // navigate returns).
            loop {
                match ws.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if text.contains("\"Page.navigate\"") {
                            for ev in &events_before_reply {
                                let _ = ws.send(Message::Text(ev.to_string())).await;
                            }
                            let _ = ws.send(Message::Text(
                                json!({"id": extract_id(&text), "result": {"frameId": "main", "loaderId": "loader-new"}}).to_string(),
                            ))
                            .await;
                            break;
                        }
                        // Any other command: answer with an id-matched empty result.
                        let _ = ws
                            .send(Message::Text(
                                json!({"id": extract_id(&text), "result": {}}).to_string(),
                            ))
                            .await;
                    }
                    Some(Ok(_)) => continue,
                    _ => break,
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });
        (format!("ws://{addr}"), handle)
    }

    fn extract_id(text: &str) -> i64 {
        serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|v| v.get("id").and_then(Value::as_i64))
            .unwrap_or(0)
    }

    fn deadline(ms: u64) -> Deadline {
        Deadline::new(ms)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_navigation_catches_dom_content_loaded_fired_before_navigate_returns() {
        let (url, server) = scripted_nav_server(vec![json!({
            "method": "Page.lifecycleEvent",
            "params": {"frameId": "main", "loaderId": "loader-new", "name": "DOMContentLoaded"}
        })])
        .await;
        let conn = CdpConnection::connect(&url, 5000).await.unwrap();
        let session = CdpSession::new("", conn.clone());

        let result =
            wait_for_explicit_navigation(&session, "https://example.test/", &deadline(500))
                .await
                .unwrap();
        assert!(result.did_navigate);
        drop(server);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn action_observer_remembers_navigation_events_fired_before_wait_starts() {
        let (url, server) = scripted_nav_server(vec![]).await;
        let conn = CdpConnection::connect(&url, 5000).await.unwrap();
        let session = CdpSession::new("", conn.clone());

        let observer = ActionNavigationObserver::new("main", "loader-old");
        observer.arm(&session);

        // Fire the events directly through the connection's router (the
        // observer is already armed).
        conn.dispatch_test(
            None,
            "Page.lifecycleEvent",
            &json!({"frameId": "main", "loaderId": "loader-new", "name": "init"}),
        );
        conn.dispatch_test(
            None,
            "Page.lifecycleEvent",
            &json!({"frameId": "main", "loaderId": "loader-new", "name": "DOMContentLoaded"}),
        );

        let result = observer.wait(&deadline(500)).await.unwrap();
        observer.dispose();
        assert!(result.did_navigate);
        drop(server);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn action_observer_ignores_iframe_lifecycle_events() {
        let (url, server) = scripted_nav_server(vec![]).await;
        let conn = CdpConnection::connect(&url, 5000).await.unwrap();
        let session = CdpSession::new("", conn.clone());

        let observer = ActionNavigationObserver::new("main", "loader-old");
        observer.arm(&session);
        conn.dispatch_test(
            None,
            "Page.lifecycleEvent",
            &json!({"frameId": "iframe", "loaderId": "loader-new", "name": "DOMContentLoaded"}),
        );

        let result = observer.wait(&deadline(75)).await.unwrap();
        observer.dispose();
        assert!(!result.did_navigate);
        drop(server);
    }

    // ── Remainder coverage via the mock browser ───────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_navigation_error_text_and_send_failure() {
        let mock = crate::test_cdp::MockCdp::start().await;
        mock.state.lock().unwrap().navigate_error_text = Some("net::ERR_X".to_string());
        let (conn, session) = {
            let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
            (conn.clone(), CdpSession::new("S-1", conn))
        };
        let result = wait_for_explicit_navigation(&session, "http://x/", &deadline(500))
            .await
            .unwrap();
        assert!(!result.did_navigate);
        assert_eq!(result.error_text.as_deref(), Some("net::ERR_X"));

        // Page.navigate send failure → Err.
        mock.state.lock().unwrap().navigate_error_text = None;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Page.navigate".to_string());
        let err = wait_for_explicit_navigation(&session, "http://x/", &deadline(500))
            .await
            .unwrap_err();
        assert!(err.contains("mock failure"), "{err}");
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_navigation_same_document_and_missing_load_event() {
        let mock = crate::test_cdp::MockCdp::start().await;
        // Same-document (no loaderId) → did_navigate + same_document.
        mock.state.lock().unwrap().navigate_same_document = true;
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let session = CdpSession::new("S-1", conn.clone());
        let result = wait_for_explicit_navigation(&session, "http://x/#f", &deadline(500))
            .await
            .unwrap();
        assert!(result.did_navigate);
        assert_eq!(result.same_document, Some(true));

        // loaderId present but DOMContentLoaded never arrives → still
        // navigated after the bounded wait (non-matching events ignored).
        {
            let mut state = mock.state.lock().unwrap();
            state.navigate_same_document = false;
            state.suppress_loaded_event = true;
        }
        let result = wait_for_explicit_navigation(&session, "http://y/", &deadline(150))
            .await
            .unwrap();
        assert!(result.did_navigate);
        assert_eq!(result.same_document, None);
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observer_ignores_non_domcontentloaded_and_same_loader() {
        let (url, server) = scripted_nav_server(vec![]).await;
        let conn = CdpConnection::connect(&url, 5000).await.unwrap();
        let session = CdpSession::new("", conn.clone());

        let observer = ActionNavigationObserver::new("main", "loader-old");
        observer.arm(&session);
        // Same loader id → not a new navigation.
        conn.dispatch_test(
            None,
            "Page.lifecycleEvent",
            &json!({"frameId": "main", "loaderId": "loader-old", "name": "DOMContentLoaded"}),
        );
        // New loader but a non-DOMContentLoaded name (recorded as new
        // loader, but loaded-set lacks it → wait reports navigation once
        // the loader switch is seen).
        conn.dispatch_test(
            None,
            "Page.lifecycleEvent",
            &json!({"frameId": "main", "loaderId": "loader-new", "name": "load"}),
        );
        // Event without loaderId → ignored entirely.
        conn.dispatch_test(
            None,
            "Page.lifecycleEvent",
            &json!({"frameId": "main", "name": "DOMContentLoaded"}),
        );
        let result = observer.wait(&deadline(200)).await.unwrap();
        observer.dispose();
        // New loader id was registered (navigation started) even though the
        // DOMContentLoaded for it never arrived within the deadline.
        assert!(result.did_navigate);
        drop(server);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observer_disposed_short_circuits_arm_and_wait() {
        let (url, server) = scripted_nav_server(vec![]).await;
        let conn = CdpConnection::connect(&url, 5000).await.unwrap();
        let session = CdpSession::new("", conn.clone());

        let observer = ActionNavigationObserver::new("main", "loader-old");
        observer.dispose();
        // arm() after dispose is a no-op.
        observer.arm(&session);
        // wait() after dispose resolves immediately with no navigation.
        let started = std::time::Instant::now();
        let result = observer.wait(&deadline(5_000)).await.unwrap();
        assert!(started.elapsed().as_millis() < 100);
        assert!(!result.did_navigate);
        drop(server);
    }

    #[test]
    fn navigation_result_default() {
        let r = NavigationResult::default();
        assert!(!r.did_navigate);
        assert!(r.new_url.is_none());
        assert!(r.error_text.is_none());
        assert!(r.same_document.is_none());
        let cloned = r.clone();
        assert!(!cloned.did_navigate);
        let _ = format!("{r:?}");
    }
}
