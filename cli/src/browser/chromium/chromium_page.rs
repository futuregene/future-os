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
