//! CDP event router — port of `cli/src/browser/chromium/cdp-event-router.ts`.
//!
//! Events are keyed by `${sessionId ?? "browser"}::${method}` so a
//! Page.loadEventFired on tab A never wakes a navigation waiter on tab B.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// `CdpEventHandler` — `(params: Value) => void`. Handlers must be cheap and
/// must not await (they run on the connection's dispatch task).
pub type CdpEventHandler = Arc<dyn Fn(&Value) + Send + Sync>;

/// `CdpEventRouter` — per-(sessionId, method) handler registry.
#[derive(Default)]
pub struct CdpEventRouter {
    handlers: Mutex<HashMap<String, Vec<CdpEventHandler>>>,
}

/// Unsubscribe handle — call `unsubscribe()` to remove the handler. Unlike a
/// Drop guard, an ignored handle leaves the handler registered (matching the
/// TS behavior where `add` returns a closure nobody has to call).
pub struct Unsubscribe {
    router: Arc<CdpEventRouter>,
    key: String,
    handler: CdpEventHandler,
}

impl Unsubscribe {
    pub fn unsubscribe(self) {
        if let Ok(mut map) = self.router.handlers.lock() {
            if let Some(list) = map.get_mut(&self.key) {
                list.retain(|h| !Arc::ptr_eq(h, &self.handler));
                if list.is_empty() {
                    map.remove(&self.key);
                }
            }
        }
    }
}

impl CdpEventRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `add(sessionId, method, handler)` — register; returns unsubscribe.
    pub fn add(
        self: &Arc<Self>,
        session_id: Option<&str>,
        method: &str,
        handler: CdpEventHandler,
    ) -> Unsubscribe {
        let key = event_key(session_id, method);
        if let Ok(mut map) = self.handlers.lock() {
            map.entry(key.clone()).or_default().push(handler.clone());
        }
        Unsubscribe {
            router: self.clone(),
            key,
            handler,
        }
    }

    /// `dispatch(sessionId, method, params)` — specific key then wildcard key.
    pub fn dispatch(&self, session_id: Option<&str>, method: &str, params: &Value) {
        let specific_key = event_key(session_id, method);
        if let Ok(map) = self.handlers.lock() {
            if let Some(list) = map.get(&specific_key) {
                for handler in list {
                    // One handler throwing must not break the others.
                    let _ =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(params)));
                }
            }

            // Wildcard: all sessions for this method.
            let wildcard_key = event_key(None, method);
            if wildcard_key != specific_key {
                if let Some(list) = map.get(&wildcard_key) {
                    for handler in list {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            handler(params)
                        }));
                    }
                }
            }
        }
    }

    /// `clearSession(sessionId)` — remove all handlers for a session.
    pub fn clear_session(&self, session_id: &str) {
        if let Ok(mut map) = self.handlers.lock() {
            map.retain(|key, _| !key.starts_with(&format!("{session_id}::")));
        }
    }

    /// `clear()` — remove ALL handlers.
    pub fn clear(&self) {
        if let Ok(mut map) = self.handlers.lock() {
            map.clear();
        }
    }
}

/// `${sessionId ?? "browser"}::${method}`.
fn event_key(session_id: Option<&str>, method: &str) -> String {
    format!("{}::{method}", session_id.unwrap_or("browser"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn dispatches_to_matching_session_and_method() {
        let router = Arc::new(CdpEventRouter::new());
        let received = Arc::new(Mutex::new(Vec::new()));
        let rec = received.clone();
        let h: CdpEventHandler = Arc::new(move |params| {
            rec.lock().unwrap().push(params.clone());
        });
        router.add(Some("session-1"), "Page.loadEventFired", h);

        router.dispatch(
            Some("session-1"),
            "Page.loadEventFired",
            &json!({"timestamp": 123}),
        );
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn does_not_dispatch_to_wrong_session() {
        let router = Arc::new(CdpEventRouter::new());
        let received = Arc::new(Mutex::new(Vec::new()));
        let rec = received.clone();
        let h: CdpEventHandler = Arc::new(move |params| {
            rec.lock().unwrap().push(params.clone());
        });
        router.add(Some("session-1"), "Page.loadEventFired", h);

        router.dispatch(
            Some("session-2"),
            "Page.loadEventFired",
            &json!({"timestamp": 456}),
        );
        assert_eq!(received.lock().unwrap().len(), 0);
    }

    #[test]
    fn does_not_dispatch_to_wrong_method() {
        let router = Arc::new(CdpEventRouter::new());
        let received = Arc::new(Mutex::new(Vec::new()));
        let rec = received.clone();
        let h: CdpEventHandler = Arc::new(move |_params| {
            rec.lock().unwrap().push(1);
        });
        router.add(Some("session-1"), "Page.loadEventFired", h);

        router.dispatch(Some("session-1"), "Page.domContentEventFired", &json!({}));
        assert_eq!(received.lock().unwrap().len(), 0);
    }

    #[test]
    fn wildcard_handlers_receive_all_sessions() {
        let router = Arc::new(CdpEventRouter::new());
        let received = Arc::new(Mutex::new(Vec::new()));
        let rec = received.clone();
        let h: CdpEventHandler = Arc::new(move |params| {
            rec.lock().unwrap().push(params.clone());
        });
        router.add(None, "Target.targetCreated", h);

        router.dispatch(
            Some("session-1"),
            "Target.targetCreated",
            &json!({"targetId": "1"}),
        );
        router.dispatch(
            Some("session-2"),
            "Target.targetCreated",
            &json!({"targetId": "2"}),
        );
        assert_eq!(received.lock().unwrap().len(), 2);
    }

    #[test]
    fn unsubscribe_stops_delivery() {
        let router = Arc::new(CdpEventRouter::new());
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let h: CdpEventHandler = Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let unsub = router.add(Some("session-1"), "Page.loadEventFired", h);
        unsub.unsubscribe();
        router.dispatch(Some("session-1"), "Page.loadEventFired", &json!({}));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn clear_session_removes_only_that_session() {
        let router = Arc::new(CdpEventRouter::new());
        let s1 = Arc::new(AtomicUsize::new(0));
        let s2 = Arc::new(AtomicUsize::new(0));
        let a = s1.clone();
        let b = s2.clone();
        router.add(
            Some("session-1"),
            "Page.loadEventFired",
            Arc::new(move |_| {
                a.fetch_add(1, Ordering::SeqCst);
            }),
        );
        router.add(
            Some("session-2"),
            "Page.loadEventFired",
            Arc::new(move |_| {
                b.fetch_add(1, Ordering::SeqCst);
            }),
        );

        router.clear_session("session-1");
        router.dispatch(Some("session-1"), "Page.loadEventFired", &json!({}));
        router.dispatch(Some("session-2"), "Page.loadEventFired", &json!({}));

        assert_eq!(s1.load(Ordering::SeqCst), 0);
        assert_eq!(s2.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clear_removes_all_handlers() {
        let router = Arc::new(CdpEventRouter::new());
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            router.add(
                Some("session-1"),
                "Page.loadEventFired",
                Arc::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                }),
            );
        }
        {
            let c = count.clone();
            router.add(
                Some("session-2"),
                "Page.loadEventFired",
                Arc::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                }),
            );
        }
        {
            let c = count.clone();
            router.add(
                None,
                "Target.targetCreated",
                Arc::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                }),
            );
        }

        router.clear();
        router.dispatch(Some("session-1"), "Page.loadEventFired", &json!({}));
        router.dispatch(Some("session-2"), "Page.loadEventFired", &json!({}));
        router.dispatch(Some("session-3"), "Target.targetCreated", &json!({}));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn one_handler_throwing_does_not_break_others() {
        let router = Arc::new(CdpEventRouter::new());
        let ok = Arc::new(Mutex::new(Vec::new()));
        let rec = ok.clone();
        router.add(
            Some("s"),
            "test",
            Arc::new(move |_| {
                panic!("boom");
            }),
        );
        router.add(
            Some("s"),
            "test",
            Arc::new(move |_| {
                rec.lock().unwrap().push("still called");
            }),
        );

        router.dispatch(Some("s"), "test", &json!({}));
        assert_eq!(ok.lock().unwrap().as_slice(), &["still called"]);
    }

    #[test]
    fn dispatch_without_any_handlers_is_a_noop() {
        let router = Arc::new(CdpEventRouter::new());
        // No handlers registered at all: specific + wildcard lookups miss.
        router.dispatch(Some("s"), "Nothing", &json!({}));
        router.dispatch(None, "Nothing", &json!({}));
    }

    #[test]
    fn dispatch_with_none_session_does_not_double_fire() {
        // sessionId None: specific key == wildcard key → one invocation.
        let router = Arc::new(CdpEventRouter::new());
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        router.add(None, "Ev", Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        router.dispatch(None, "Ev", &json!({}));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unsubscribe_with_siblings_keeps_key_and_double_unsubscribe_is_noop() {
        let router = Arc::new(CdpEventRouter::new());
        let count = Arc::new(AtomicUsize::new(0));
        let c1 = count.clone();
        let c2 = count.clone();
        let first = router.add(Some("s"), "Ev", Arc::new(move |_| {
            c1.fetch_add(1, Ordering::SeqCst);
        }));
        router.add(Some("s"), "Ev", Arc::new(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
        }));
        // Removing the first leaves the second registered under the key.
        first.unsubscribe();
        router.dispatch(Some("s"), "Ev", &json!({}));
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // Unsubscribing a key that no longer exists is a no-op.
        let lone_router = Arc::new(CdpEventRouter::new());
        let h = lone_router.add(Some("x"), "Ev", Arc::new(|_| {}));
        h.unsubscribe();
        // (key removed above) — build a fresh handle against the same key.
        let h2 = lone_router.add(Some("x"), "Ev", Arc::new(|_| {}));
        h2.unsubscribe();
        lone_router.dispatch(Some("x"), "Ev", &json!({}));
    }

    #[test]
    fn clear_session_without_matches_keeps_others() {
        let router = Arc::new(CdpEventRouter::new());
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        router.add(Some("keep"), "Ev", Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        router.clear_session("absent");
        router.dispatch(Some("keep"), "Ev", &json!({}));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn event_key_shapes() {
        assert_eq!(event_key(None, "M"), "browser::M");
        assert_eq!(event_key(Some("s"), "M"), "s::M");
    }
}
