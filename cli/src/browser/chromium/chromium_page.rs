//! Chromium page management — port of
//! `cli/src/browser/chromium/chromium-page.ts`.
//!
//! Target discovery, tab CRUD, active page tracking via
//! Target.setDiscoverTargets + Target.targetCreated (NOT Target.setAutoAttach).
//! Only attaches to type="page" targets.

use super::cdp_connection::{CdpConnection, CdpSendError, CdpSession};
use super::target_registry::AttachedTarget;
use crate::browser::tab_order::{insert_new_page, reconcile_page_order, remove_page};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// `ChromiumPage`.
#[derive(Debug, Clone)]
pub struct ChromiumPage {
    pub target_id: String,
    pub session_id: String,
    pub r#type: String,
    pub url: String,
    pub title: String,
}

/// Shared page state behind a mutex (event handlers run on the connection's
/// dispatch task and must mutate it). The pages map preserves insertion
/// order (JS `Map` semantics — target discovery order is meaningful).
#[derive(Default)]
struct PageData {
    pages: indexmap::IndexMap<String, ChromiumPage>,
    active_page_id: Option<String>,
    tab_order: Vec<String>,
}

/// `ChromiumPageManager`.
pub struct ChromiumPageManager {
    data: Arc<Mutex<PageData>>,
    browser_session: CdpSession,
    connection: Arc<CdpConnection>,
    _unsubs: Mutex<Vec<crate::browser::chromium::cdp_event_router::Unsubscribe>>,
    disposed: Arc<AtomicBool>,
}

impl ChromiumPageManager {
    pub fn new(browser_session: CdpSession, connection: Arc<CdpConnection>) -> Self {
        ChromiumPageManager {
            data: Arc::new(Mutex::new(PageData::default())),
            browser_session,
            connection,
            _unsubs: Mutex::new(Vec::new()),
            disposed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `initialize(existingTabOrder?, activePageId?)`.
    pub async fn initialize(
        &mut self,
        existing_tab_order: Option<&[String]>,
        active_page_id: Option<&str>,
    ) -> Result<(), String> {
        self.browser_session
            .send(
                "Target.setDiscoverTargets",
                Some(
                    &json!({"discover": true, "filter": [{"type": "page"}]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;

        // Target.targetCreated
        {
            let data = self.data.clone();
            let h: crate::browser::chromium::cdp_event_router::CdpEventHandler =
                std::sync::Arc::new(move |event: &Value| {
                    let info = event.get("targetInfo").and_then(Value::as_object);
                    if let Some(info) = info {
                        let r#type = info
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if r#type != "page" {
                            return;
                        }
                        let target_id = info
                            .get("targetId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let url = info
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let title = info
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if let Ok(mut d) = data.lock() {
                            if !d.pages.contains_key(&target_id) {
                                d.pages.insert(
                                    target_id.clone(),
                                    ChromiumPage {
                                        target_id,
                                        session_id: String::new(),
                                        r#type,
                                        url,
                                        title,
                                    },
                                );
                            }
                        }
                    }
                });
            self._unsubs
                .lock()
                .unwrap()
                .push(self.browser_session.on("Target.targetCreated", h));
        }

        // Target.targetInfoChanged
        {
            let data = self.data.clone();
            let h: crate::browser::chromium::cdp_event_router::CdpEventHandler =
                std::sync::Arc::new(move |event: &Value| {
                    let info = event.get("targetInfo").and_then(Value::as_object);
                    if let Some(info) = info {
                        let target_id = info
                            .get("targetId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let url = info
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let title = info
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if let Ok(mut d) = data.lock() {
                            if let Some(existing) = d.pages.get_mut(&target_id) {
                                existing.url = url;
                                existing.title = title;
                            }
                        }
                    }
                });
            self._unsubs
                .lock()
                .unwrap()
                .push(self.browser_session.on("Target.targetInfoChanged", h));
        }

        // Unified cleanup on Target.targetDestroyed
        {
            let data = self.data.clone();
            let connection = self.connection.clone();
            let disposed = self.disposed.clone();
            let h: crate::browser::chromium::cdp_event_router::CdpEventHandler =
                std::sync::Arc::new(move |event: &Value| {
                    let target_id = event
                        .get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let attached = connection.detach_target_by_target_id(&target_id);
                    if let Some(attached) = attached {
                        connection.reject_pending_for_session(
                            &attached.session_id,
                            CdpSendError::Protocol(
                                crate::browser::chromium::cdp_connection::CdpError {
                                    code: -1,
                                    message: format!("Target {target_id} destroyed"),
                                },
                            ),
                        );
                    }
                    if let Ok(mut d) = data.lock() {
                        d.pages.shift_remove(&target_id);
                        d.tab_order = remove_page(&d.tab_order, &target_id);
                        if d.active_page_id.as_deref() == Some(target_id.as_str()) {
                            d.active_page_id = d.tab_order.last().cloned();
                        }
                    }
                    let _ = disposed;
                });
            self._unsubs
                .lock()
                .unwrap()
                .push(self.browser_session.on("Target.targetDestroyed", h));
        }

        // Get existing targets.
        let targets = self
            .browser_session
            .send(
                "Target.getTargets",
                Some(
                    &json!({"filter": [{"type": "page"}]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;

        let target_infos = targets
            .get("targetInfos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for info in &target_infos {
            let r#type = info
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if r#type == "page" {
                let target_id = info
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let url = info
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = info
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.attach_to_target(&target_id, &r#type, &url, &title)
                    .await?;
            }
        }

        let page_ids: Vec<String> = {
            let d = self.data.lock().unwrap();
            d.pages.keys().cloned().collect()
        };
        let ordered = reconcile_page_order(existing_tab_order, &page_ids);
        {
            let mut d = self.data.lock().unwrap();
            d.tab_order = ordered;
            // Restore active page from config, or default to last.
            if let Some(active) = active_page_id {
                if d.pages.contains_key(active) {
                    d.active_page_id = Some(active.to_string());
                }
            }
        }
        Ok(())
    }

    async fn attach_to_target(
        &self,
        target_id: &str,
        r#type: &str,
        url: &str,
        title: &str,
    ) -> Result<(), String> {
        let result = self
            .browser_session
            .send(
                "Target.attachToTarget",
                Some(
                    &json!({"targetId": target_id, "flatten": true})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        if let Ok(mut d) = self.data.lock() {
            d.pages.insert(
                target_id.to_string(),
                ChromiumPage {
                    target_id: target_id.to_string(),
                    session_id,
                    r#type: r#type.to_string(),
                    url: url.to_string(),
                    title: title.to_string(),
                },
            );
        }
        Ok(())
    }

    // ── Tab management ────────────────────────────────────────────────

    /// `createPage(url = "about:blank")`.
    pub async fn create_page(&self, url: &str) -> Result<(String, ChromiumPage), String> {
        let result = self
            .browser_session
            .send(
                "Target.createTarget",
                Some(&json!({"url": url}).as_object().unwrap().clone()),
            )
            .await
            .map_err(|e| e.to_string())?;
        let target_id = result
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // Wait for target discovery.
        let deadline = crate::utils::time::now_millis() + 5000;
        let mut page: Option<ChromiumPage> = None;
        while crate::utils::time::now_millis() < deadline {
            page = {
                let d = self.data.lock().unwrap();
                d.pages.get(&target_id).cloned()
            };
            if page.is_some() {
                break;
            }
            crate::utils::time::sleep(50).await;
        }
        let mut page =
            page.ok_or_else(|| format!("Target {target_id} not discovered within timeout"))?;

        // Attach if not yet attached.
        if page.session_id.is_empty() {
            self.attach_to_target(&page.target_id, &page.r#type, &page.url, &page.title)
                .await?;
            page = {
                let d = self.data.lock().unwrap();
                d.pages.get(&target_id).cloned().unwrap()
            };
        }

        {
            let mut d = self.data.lock().unwrap();
            d.tab_order = insert_new_page(&d.tab_order, &target_id);
        }
        Ok((target_id, page))
    }

    /// `closePage(targetId)`.
    pub async fn close_page(&self, target_id: &str) -> Result<(), String> {
        self.browser_session
            .send(
                "Target.closeTarget",
                Some(&json!({"targetId": target_id}).as_object().unwrap().clone()),
            )
            .await
            .map_err(|e| e.to_string())?;
        // Cleanup is handled by the Target.targetDestroyed handler.
        Ok(())
    }

    /// `activatePage(targetId)`.
    pub async fn activate_page(&self, target_id: &str) -> Result<(), String> {
        self.browser_session
            .send(
                "Target.activateTarget",
                Some(&json!({"targetId": target_id}).as_object().unwrap().clone()),
            )
            .await
            .map_err(|e| e.to_string())?;
        if let Ok(mut d) = self.data.lock() {
            d.active_page_id = Some(target_id.to_string());
        }
        Ok(())
    }

    // ── Queries ───────────────────────────────────────────────────────

    pub fn get_pages(&self) -> Vec<ChromiumPage> {
        let d = self.data.lock().unwrap();
        d.tab_order
            .iter()
            .filter_map(|id| d.pages.get(id).cloned())
            .collect()
    }

    pub fn get_page(&self, target_id: &str) -> Option<ChromiumPage> {
        self.data.lock().unwrap().pages.get(target_id).cloned()
    }

    pub fn get_active_page(&self) -> Option<ChromiumPage> {
        let d = self.data.lock().unwrap();
        if let Some(active) = &d.active_page_id {
            if let Some(page) = d.pages.get(active) {
                return Some(page.clone());
            }
        }
        let ordered: Vec<&ChromiumPage> = d
            .tab_order
            .iter()
            .filter_map(|id| d.pages.get(id))
            .collect();
        ordered.last().cloned().cloned()
    }

    pub fn get_active_page_id(&self) -> Option<String> {
        self.get_active_page().map(|p| p.target_id)
    }

    pub fn get_tab_order(&self) -> Vec<String> {
        self.data.lock().unwrap().tab_order.clone()
    }

    pub fn set_active_page_id(&self, page_id: &str) {
        if let Ok(mut d) = self.data.lock() {
            if d.pages.contains_key(page_id) {
                d.active_page_id = Some(page_id.to_string());
            }
        }
    }

    /// Update page info after navigation (open() refreshes title/url).
    pub fn update_page(&self, target_id: &str, url: &str, title: &str) {
        if let Ok(mut d) = self.data.lock() {
            if let Some(page) = d.pages.get_mut(target_id) {
                page.url = url.to_string();
                page.title = title.to_string();
            }
        }
    }

    /// Record an attached session id for a page (activePageSession attach).
    pub fn set_session_id(&self, target_id: &str, session_id: &str) {
        if let Ok(mut d) = self.data.lock() {
            if let Some(page) = d.pages.get_mut(target_id) {
                page.session_id = session_id.to_string();
            }
        }
    }

    /// Record an attached target (used by activePageSession).
    pub fn register_attached(&self, target: AttachedTarget) {
        self.connection.register_target(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_cdp::{target, MockCdp};
    use serde_json::json;

    /// Start a mock with two pages and a connected page manager.
    async fn manager_over_mock() -> (MockCdp, std::sync::Arc<CdpConnection>, ChromiumPageManager) {
        let mock = MockCdp::start_with(
            vec![
                target("T-1", "http://one/", "One"),
                target("T-2", "http://two/", "Two"),
            ],
            "Chrome/126.0.0.0",
        )
        .await;
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let browser = CdpSession::new("", conn.clone());
        let mgr = ChromiumPageManager::new(browser, conn.clone());
        (mock, conn, mgr)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initialize_discovers_attaches_and_orders_pages() {
        let (mock, conn, mut mgr) = manager_over_mock().await;
        mgr.initialize(None, None).await.unwrap();

        let pages = mgr.get_pages();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].target_id, "T-1");
        assert_eq!(pages[0].url, "http://one/");
        assert_eq!(pages[1].title, "Two");
        // Both attached (session ids assigned by the mock).
        assert!(!pages[0].session_id.is_empty());
        assert!(mgr.get_page("T-2").is_some());
        assert!(mgr.get_page("nope").is_none());
        // No active configured → last in order.
        assert_eq!(mgr.get_active_page_id().as_deref(), Some("T-2"));
        assert_eq!(
            mgr.get_tab_order(),
            vec!["T-1".to_string(), "T-2".to_string()]
        );

        // Target.setDiscoverTargets + getTargets + 2 attaches happened.
        assert_eq!(mock.commands_of("Target.setDiscoverTargets").len(), 1);
        assert_eq!(mock.commands_of("Target.attachToTarget").len(), 2);
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initialize_restores_tab_order_and_active_page() {
        let (mock, conn, mut mgr) = manager_over_mock().await;
        mgr.initialize(Some(&["T-2".to_string(), "T-1".to_string()]), Some("T-1"))
            .await
            .unwrap();
        assert_eq!(
            mgr.get_tab_order(),
            vec!["T-2".to_string(), "T-1".to_string()]
        );
        assert_eq!(mgr.get_active_page_id().as_deref(), Some("T-1"));

        // Unknown active id is ignored → falls back to last.
        let (_m2, conn2, mut mgr2) = manager_over_mock().await;
        mgr2.initialize(None, Some("ghost")).await.unwrap();
        assert_eq!(mgr2.get_active_page_id().as_deref(), Some("T-2"));
        conn.disconnect().await;
        conn2.disconnect().await;
        drop(mock);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initialize_propagates_send_failures() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Target.setDiscoverTargets".to_string());
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let browser = CdpSession::new("", conn.clone());
        let mut mgr = ChromiumPageManager::new(browser, conn.clone());
        let err = mgr.initialize(None, None).await.unwrap_err();
        assert!(err.contains("mock failure"), "{err}");

        // getTargets failure.
        let mock2 = MockCdp::start().await;
        mock2
            .state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Target.getTargets".to_string());
        let conn2 = CdpConnection::connect(&mock2.ws_url, 5_000).await.unwrap();
        let browser2 = CdpSession::new("", conn2.clone());
        let mut mgr2 = ChromiumPageManager::new(browser2, conn2.clone());
        assert!(mgr2.initialize(None, None).await.is_err());

        // attachToTarget failure (targets exist).
        mock2.state.lock().unwrap().fail_methods.clear();
        mock2
            .state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Target.attachToTarget".to_string());
        let mut mgr3 = ChromiumPageManager::new(CdpSession::new("", conn2.clone()), conn2.clone());
        assert!(mgr3.initialize(None, None).await.is_err());
        conn.disconnect().await;
        conn2.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn target_events_update_state() {
        let (_mock, conn, mut mgr) = manager_over_mock().await;
        mgr.initialize(None, None).await.unwrap();

        // targetCreated for a NON-page target is ignored.
        conn.dispatch_test(
            None,
            "Target.targetCreated",
            &json!({"targetInfo": {"targetId": "W-1", "type": "worker", "url": "w", "title": "w"}}),
        );
        // targetCreated for a page adds it; a duplicate id does not.
        conn.dispatch_test(
            None,
            "Target.targetCreated",
            &json!({"targetInfo": {"targetId": "T-3", "type": "page", "url": "http://three/", "title": "Three"}}),
        );
        conn.dispatch_test(
            None,
            "Target.targetCreated",
            &json!({"targetInfo": {"targetId": "T-3", "type": "page", "url": "dup", "title": "dup"}}),
        );
        // targetCreated fills the pages map (tab order is reconciled
        // separately); the duplicate id was ignored.
        assert_eq!(mgr.get_pages().len(), 2);
        assert_eq!(mgr.get_page("T-3").unwrap().url, "http://three/");
        assert!(mgr.get_page("W-1").is_none());

        // targetInfoChanged updates url/title of an existing page.
        conn.dispatch_test(
            None,
            "Target.targetInfoChanged",
            &json!({"targetInfo": {"targetId": "T-3", "url": "http://moved/", "title": "Moved"}}),
        );
        assert_eq!(mgr.get_page("T-3").unwrap().url, "http://moved/");
        assert_eq!(mgr.get_page("T-3").unwrap().title, "Moved");

        // targetDestroyed removes the page and fixes the active pointer.
        mgr.set_active_page_id("T-3");
        assert_eq!(mgr.get_active_page_id().as_deref(), Some("T-3"));
        conn.dispatch_test(None, "Target.targetDestroyed", &json!({"targetId": "T-3"}));
        assert!(mgr.get_page("T-3").is_none());
        assert_eq!(mgr.get_active_page_id().as_deref(), Some("T-2"));
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn target_destroyed_rejects_pending_for_attached_session() {
        let (mock, conn, mut mgr) = manager_over_mock().await;
        mgr.initialize(None, None).await.unwrap();

        // Register the target in the connection registry, then park a
        // pending request on its session.
        conn.register_target(AttachedTarget {
            target_id: "T-1".to_string(),
            session_id: "SID-T1".to_string(),
            r#type: "page".to_string(),
        });
        mock.state
            .lock()
            .unwrap()
            .no_reply_methods
            .insert("Never.answered".to_string());
        let pending_conn = conn.clone();
        let pending = tokio::spawn(async move {
            pending_conn
                .send_with_timeout("Never.answered", None, Some("SID-T1"), 60_000)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        conn.dispatch_test(None, "Target.targetDestroyed", &json!({"targetId": "T-1"}));
        let err = pending.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Target T-1 destroyed");
        assert!(mgr.get_page("T-1").is_none());
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_close_activate_page_flows() {
        let (mock, conn, mut mgr) = manager_over_mock().await;
        mgr.initialize(None, None).await.unwrap();

        let (target_id, page) = mgr.create_page("http://new/").await.unwrap();
        assert!(target_id.starts_with("T-"));
        assert_eq!(page.url, "http://new/");
        assert!(mgr.get_tab_order().last().unwrap() == &target_id);
        assert_eq!(mock.commands_of("Target.createTarget").len(), 1);

        // create_page when the page is ALREADY discovered+attached is a
        // no-op attach-wise.
        let (again_id, _) = mgr.create_page("http://again/").await.unwrap();
        assert!(mgr.get_page(&again_id).is_some());

        // activate_page sets the active pointer.
        mgr.activate_page("T-1").await.unwrap();
        assert_eq!(mgr.get_active_page_id().as_deref(), Some("T-1"));

        // close_page → targetDestroyed cleanup.
        mgr.close_page(&target_id).await.unwrap();
        for _ in 0..50 {
            if mgr.get_page(&target_id).is_none() {
                break;
            }
            crate::utils::time::sleep(20).await;
        }
        assert!(mgr.get_page(&target_id).is_none());

        // activate/close send failures propagate.
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Target.activateTarget".to_string());
        assert!(mgr.activate_page("T-1").await.is_err());
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Target.closeTarget".to_string());
        assert!(mgr.close_page("T-1").await.is_err());
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_page_times_out_when_target_never_discovered() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().suppress_target_created = true;
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let mut mgr = ChromiumPageManager::new(CdpSession::new("", conn.clone()), conn.clone());
        mgr.initialize(None, None).await.unwrap();
        let started = std::time::Instant::now();
        let err = mgr.create_page("http://lost/").await.unwrap_err();
        assert!(err.contains("not discovered within timeout"), "{err}");
        assert!(started.elapsed().as_secs() >= 5);
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn query_and_mutation_helpers() {
        let (_mock, conn, mut mgr) = manager_over_mock().await;
        mgr.initialize(None, None).await.unwrap();

        // get_active_page: active id pointing at a REMOVED page falls back
        // to the last ordered page.
        {
            let mut d = mgr.data.lock().unwrap();
            d.active_page_id = Some("ghost".to_string());
        }
        assert_eq!(mgr.get_active_page().unwrap().target_id, "T-2");

        // Empty manager → no active page.
        let (_m2, conn2, mgr2) = {
            let mock2 = MockCdp::start_with(vec![], "Chrome/126").await;
            let c = CdpConnection::connect(&mock2.ws_url, 5_000).await.unwrap();
            let m = ChromiumPageManager::new(CdpSession::new("", c.clone()), c.clone());
            (mock2, c, m)
        };
        assert!(mgr2.get_active_page().is_none());
        assert!(mgr2.get_active_page_id().is_none());

        // set_active_page_id ignores unknown ids.
        mgr.set_active_page_id("nope");
        assert_eq!(mgr.get_active_page_id().as_deref(), Some("T-2"));

        // update_page / set_session_id mutate only existing pages.
        mgr.update_page("T-1", "http://u/", "U");
        assert_eq!(mgr.get_page("T-1").unwrap().title, "U");
        mgr.update_page("nope", "x", "x");
        mgr.set_session_id("T-1", "SID-X");
        assert_eq!(mgr.get_page("T-1").unwrap().session_id, "SID-X");
        mgr.set_session_id("nope", "SID-Y");

        // register_attached proxies to the connection registry.
        mgr.register_attached(AttachedTarget {
            target_id: "T-1".to_string(),
            session_id: "SID-X".to_string(),
            r#type: "page".to_string(),
        });
        assert_eq!(
            mgr.connection
                .get_target_by_session_id("SID-X")
                .map(|t| t.target_id),
            Some("T-1".to_string())
        );
        conn.disconnect().await;
        conn2.disconnect().await;
    }
}
