//! ChromiumSession — BrowserSession implementation for Chrome/Edge/Chromium
//! via CDP. Port of `cli/src/browser/chromium/chromium-session.ts`.

use super::cdp_connection::{CdpConnection, CdpSession};
use super::chromium_console_hook::{install_console_hook, with_temporary_preload};
use super::chromium_endpoint::resolve_cdp_endpoint;
use super::chromium_navigation::{
    wait_for_explicit_navigation, ActionNavigationObserver, NavigationResult,
};
use super::chromium_page::ChromiumPageManager;
use super::chromium_screenshot::capture_screenshot;
use super::target_registry::AttachedTarget;
use crate::browser::backend::BrowserSessionParams;
use crate::browser::backend::{
    BrowserSession, CaptureScreenshotOptions, ClickOptions, Deadline, EvaluateRequest,
    InternalActionResult, InternalPageInfo, InternalTabInfo, InternalTabsResult,
    InternalTypeResult, OpenPageOptions, PressOptions, ResolvedTarget, TabsAction, TypeOptions,
};
use crate::browser::errors::{element_not_found_error, element_not_interactable_error};
use crate::browser::input::parse_key;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::Arc;

// ── Element check script ──────────────────────────────────────────

const ELEMENT_CHECK_SCRIPT: &str = r#"function(selector) {
  var element = document.querySelector(selector);
  if (!element) return { exists: false };
  var rect = element.getBoundingClientRect();
  var style = getComputedStyle(element);
  var visible = rect.width > 0 &&
    rect.height > 0 &&
    style.visibility !== 'hidden' &&
    style.display !== 'none' &&
    Number(style.opacity || '1') > 0;
  var disabled = !!(element.disabled);
  return {
    exists: true,
    connected: element.isConnected,
    visible: visible,
    disabled: disabled,
    box: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    obscured: false,
  };
}"#;

const SCROLL_INTO_VIEW_SCRIPT: &str = r#"function(selector) {
  var element = document.querySelector(selector);
  if (element) {
    element.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    var rect = element.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  }
  return null;
}"#;

// ── Page session ───────────────────────────────────────────────────

#[derive(Clone)]
struct PageSession {
    session: CdpSession,
    page_id: String,
    main_frame_id: String,
}

// ── ChromiumSession ────────────────────────────────────────────────

/// `ChromiumSession`.
pub struct ChromiumSession {
    params: BrowserSessionParams,
    connection: Option<Arc<CdpConnection>>,
    browser_sess: Option<CdpSession>,
    page_mgr: Option<ChromiumPageManager>,
    active_ps: Option<PageSession>,
    action_timeout_ms: u64,
    navigation_timeout_ms: u64,
    init_tab_order: Option<Vec<String>>,
    init_active_page_id: Option<String>,
}

impl ChromiumSession {
    pub fn new(params: BrowserSessionParams) -> Self {
        let timeouts = params.timeouts();
        let (init_tab_order, init_active_page_id) = match &params {
            BrowserSessionParams::Cdp {
                active_page_id,
                init_tab_order,
                ..
            } => (init_tab_order.clone(), active_page_id.clone()),
            BrowserSessionParams::Webdriver { .. } => (None, None),
        };
        ChromiumSession {
            params,
            connection: None,
            browser_sess: None,
            page_mgr: None,
            active_ps: None,
            action_timeout_ms: timeouts.action_timeout_ms,
            navigation_timeout_ms: timeouts.navigation_timeout_ms,
            init_tab_order,
            init_active_page_id,
        }
    }

    // ── Init ──────────────────────────────────────────────────────────

    async fn init(&mut self) -> Result<(), String> {
        if let Some(conn) = &self.connection {
            if conn.is_connected() && self.page_mgr.is_some() {
                return Ok(());
            }
        }

        if self.params.protocol() != "cdp" {
            return Err("ChromiumSession requires CDP protocol".to_string());
        }

        let endpoint_info = resolve_cdp_endpoint(self.params.endpoint(), 5_000).await?;

        let connection = CdpConnection::connect(&endpoint_info.web_socket_debugger_url, 10_000)
            .await
            .map_err(|e| e.to_string())?;

        let browser_sess = CdpSession::new("", connection.clone());

        let mut page_mgr = ChromiumPageManager::new(browser_sess.clone(), connection.clone());
        page_mgr
            .initialize(
                self.init_tab_order.as_deref(),
                self.init_active_page_id.as_deref(),
            )
            .await?;

        self.connection = Some(connection);
        self.browser_sess = Some(browser_sess);
        self.page_mgr = Some(page_mgr);
        Ok(())
    }

    async fn active_page_session(&mut self) -> Result<PageSession, String> {
        self.init().await?;
        let connection = self.connection.clone().unwrap();
        let browser_sess = self.browser_sess.clone().unwrap();
        let page_mgr = self.page_mgr.as_mut().unwrap();

        let mut page = page_mgr.get_active_page();
        if page.is_none() {
            let created = page_mgr.create_page("about:blank").await?;
            page = Some(created.1);
        }
        let page = page.unwrap();

        let session = if !page.session_id.is_empty() {
            CdpSession::new(&page.session_id, connection.clone())
        } else {
            let attach_result = browser_sess
                .send(
                    "Target.attachToTarget",
                    Some(
                        &json!({"targetId": page.target_id, "flatten": true})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                )
                .await
                .map_err(|e| e.to_string())?;
            let sid = attach_result
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            page_mgr.set_session_id(&page.target_id, &sid);
            CdpSession::new(&sid, connection.clone())
        };

        session
            .send("Page.enable", None)
            .await
            .map_err(|e| e.to_string())?;
        session
            .send("Runtime.enable", None)
            .await
            .map_err(|e| e.to_string())?;
        session
            .send(
                "Page.setLifecycleEventsEnabled",
                Some(&json!({"enabled": true}).as_object().unwrap().clone()),
            )
            .await
            .map_err(|e| e.to_string())?;

        connection.register_target(AttachedTarget {
            target_id: page.target_id.clone(),
            session_id: session.session_id.clone(),
            r#type: "page".to_string(),
        });

        let (main_frame_id, _loader_id) = get_main_frame_state(&session).await?;

        let ps = PageSession {
            session,
            page_id: page.target_id,
            main_frame_id,
        };
        self.active_ps = Some(ps.clone());
        Ok(ps)
    }

    fn dispose_page_session(&mut self) {
        self.active_ps = None;
    }

    // ── Evaluate helpers ──────────────────────────────────────────────

    async fn evaluate_expression<T>(&self, session: &CdpSession, expression: &str) -> T
    where
        T: for<'de> serde::Deserialize<'de> + Default,
    {
        let params = json!({ "expression": expression, "returnByValue": true });
        match session.send("Runtime.evaluate", params.as_object()).await {
            Ok(raw) => {
                let value = raw
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .cloned()
                    .unwrap_or(Value::Null);
                serde_json::from_value::<T>(value).unwrap_or_default()
            }
            Err(_) => T::default(),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────

    async fn wait_for_actionable(
        &self,
        session: &CdpSession,
        selector: &str,
        deadline: &Deadline,
    ) -> Result<(), String> {
        loop {
            if deadline.expired() {
                break;
            }
            let expression = format!("({ELEMENT_CHECK_SCRIPT})({})", json!(selector));
            let params = json!({ "expression": expression, "returnByValue": true });
            let check: Value = match session.send("Runtime.evaluate", params.as_object()).await {
                Ok(raw) => raw
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .cloned()
                    .unwrap_or(Value::Null),
                Err(_) => Value::Null,
            };

            let exists = check
                .get("exists")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !exists {
                crate::utils::time::sleep(100).await;
                continue;
            }
            let connected = check
                .get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let visible = check
                .get("visible")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !connected || !visible {
                crate::utils::time::sleep(100).await;
                continue;
            }
            let disabled = check
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if disabled {
                return Err(
                    element_not_interactable_error(selector, "element is disabled").to_string(),
                );
            }
            return Ok(());
        }
        Err(element_not_found_error(selector).to_string())
    }

    async fn focus_and_clear(&self, session: &CdpSession, selector: &str) -> Result<(), String> {
        let expression = format!(
            "(function(){{var el=document.querySelector({});if(el){{el.focus();el.select()}}}})()",
            json!(selector)
        );
        session
            .send(
                "Runtime.evaluate",
                Some(
                    &json!({"expression": expression})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `dispatchEnter` — rawKeyDown + char(\r) to trigger form submission.
    async fn dispatch_enter(&self, session: &CdpSession) -> Result<(), String> {
        let key_down = json!({
            "type": "rawKeyDown", "key": "Enter", "code": "Enter",
            "windowsVirtualKeyCode": 13, "nativeVirtualKeyCode": 36,
        });
        let char = json!({
            "type": "char", "key": "Enter", "code": "Enter", "text": "\r",
            "windowsVirtualKeyCode": 13, "nativeVirtualKeyCode": 36,
        });
        let key_up = json!({
            "type": "keyUp", "key": "Enter", "code": "Enter",
            "windowsVirtualKeyCode": 13, "nativeVirtualKeyCode": 36,
        });
        session
            .send("Input.dispatchKeyEvent", key_down.as_object())
            .await
            .map_err(|e| e.to_string())?;
        session
            .send("Input.dispatchKeyEvent", char.as_object())
            .await
            .map_err(|e| e.to_string())?;
        session
            .send("Input.dispatchKeyEvent", key_up.as_object())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_loader_id(&self, session: &CdpSession) -> Result<String, String> {
        let result = session
            .send("Page.getFrameTree", None)
            .await
            .map_err(|e| e.to_string())?;
        Ok(result
            .get("frameTree")
            .and_then(|t| t.get("frame"))
            .and_then(|f| f.get("loaderId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    /// Random suffix for the click-state key (`Math.random().toString(36)`).
    fn random_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("{nanos:x}")
    }
}

#[async_trait]
impl BrowserSession for ChromiumSession {
    fn kind(&self) -> &'static str {
        "chromium"
    }

    fn protocol(&self) -> &'static str {
        "cdp"
    }

    // ── Open ──────────────────────────────────────────────────────────

    async fn open(
        &mut self,
        url: &str,
        _options: OpenPageOptions,
    ) -> Result<InternalPageInfo, String> {
        let ps = self.active_page_session().await?;
        let result = async {
            install_console_hook(&ps.session).await;

            let deadline = Deadline::new(self.navigation_timeout_ms);

            let nav = with_temporary_preload(&ps.session, async {
                wait_for_explicit_navigation(&ps.session, url, &deadline).await
            })
            .await;

            let nav = nav?;
            if let Some(error_text) = nav.error_text {
                return Err(format!("Navigation failed: {error_text}"));
            }

            let title: String = self
                .evaluate_expression(&ps.session, "document.title")
                .await;
            let final_url: String = self.evaluate_expression(&ps.session, "location.href").await;

            self.init().await?;
            let page_mgr = self.page_mgr.as_mut().unwrap();
            page_mgr.update_page(&ps.page_id, &final_url, &title);

            Ok(InternalPageInfo {
                page_id: ps.page_id.clone(),
                title,
                url: final_url,
            })
        }
        .await;

        self.dispose_page_session();
        result
    }

    // ── Click ─────────────────────────────────────────────────────────

    async fn click(
        &mut self,
        target: &ResolvedTarget,
        options: ClickOptions,
    ) -> Result<InternalActionResult, String> {
        let ps = self.active_page_session().await?;
        let result = async {
            install_console_hook(&ps.session).await;

            let timeout_ms = options.timeout_ms.unwrap_or(self.action_timeout_ms);

            self.wait_for_actionable(&ps.session, &target.selector, &Deadline::new(timeout_ms))
                .await?;

            let scroll_expr = format!("({SCROLL_INTO_VIEW_SCRIPT})({})", json!(target.selector));
            let box_: Option<Value> = self.evaluate_expression(&ps.session, &scroll_expr).await;
            let center = match box_ {
                Some(b) => {
                    let x = b.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                    let y = b.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                    let w = b.get("width").and_then(Value::as_f64).unwrap_or(0.0);
                    let h = b.get("height").and_then(Value::as_f64).unwrap_or(0.0);
                    ((x + w / 2.0).round() as i64, (y + h / 2.0).round() as i64)
                }
                None => (0, 0),
            };

            let nav_deadline = Deadline::new(self.navigation_timeout_ms);
            let current_loader = self.get_loader_id(&ps.session).await.unwrap_or_default();
            let nav_observer = ActionNavigationObserver::new(&ps.main_frame_id, &current_loader);
            nav_observer.arm(&ps.session);

            // Capture default-action metadata before mouse dispatch. A
            // successful navigation destroys the element and its execution
            // context.
            let click_state_key = format!(
                "__futureClickState_{}_{}",
                crate::utils::time::now_millis(),
                Self::random_suffix()
            );
            let key_json = json!(click_state_key);
            let selector_json = json!(target.selector);
            let meta_script = format!(
                r#"(() => {{
          const el = document.querySelector({selector_json});
          const anchor = el?.closest?.('a[href]');
          const submitter = el?.closest?.('button, input[type="submit"], input[type="image"]');
          const state = {{ defaultPrevented: false, submitSeen: false }};
          Object.defineProperty(window, {key_json}, {{
            value: state,
            configurable: true,
          }});
          window.addEventListener('click', (event) => {{
            if (el && (event.target === el || el.contains(event.target))) {{
              state.defaultPrevented = event.defaultPrevented;
            }}
          }}, {{ once: true }});
          submitter?.form?.addEventListener('submit', () => {{
            state.submitSeen = true;
          }}, {{ capture: true, once: true }});
          return {{
            href: anchor?.href || null,
            hasSubmitter: Boolean(submitter?.form && submitter?.type !== 'button'),
          }};
        }})()"#
            );
            let meta: Value = self.evaluate_expression(&ps.session, &meta_script).await;
            let meta_href = meta.get("href").and_then(Value::as_str).map(str::to_string);
            let meta_has_submitter = meta
                .get("hasSubmitter")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            with_temporary_preload(&ps.session, async {
                let moved = json!({"type": "mouseMoved", "x": center.0, "y": center.1});
                ps.session
                    .send("Input.dispatchMouseEvent", moved.as_object())
                    .await
                    .map_err(|e| e.to_string())?;
                let pressed = json!({
                    "type": "mousePressed", "x": center.0, "y": center.1,
                    "button": "left", "clickCount": 1,
                });
                ps.session
                    .send("Input.dispatchMouseEvent", pressed.as_object())
                    .await
                    .map_err(|e| e.to_string())?;
                let released = json!({
                    "type": "mouseReleased", "x": center.0, "y": center.1,
                    "button": "left", "clickCount": 1,
                });
                ps.session
                    .send("Input.dispatchMouseEvent", released.as_object())
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
            .await?;

            // First accept the browser's native default action; fall back
            // explicitly only when no navigation was observed.
            let mut nav_result: NavigationResult = nav_observer
                .wait(&nav_deadline)
                .await
                .unwrap_or(NavigationResult {
                    did_navigate: false,
                    ..Default::default()
                });

            let mut event_state = (false, false); // (defaultPrevented, submitSeen)
            if !nav_result.did_navigate {
                let state_script = format!(
                    r#"(() => {{
            const state = window[{key_json}] || {{}};
            delete window[{key_json}];
            return {{
              defaultPrevented: Boolean(state.defaultPrevented),
              submitSeen: Boolean(state.submitSeen),
            }};
          }})()"#
                );
                let state: Value = self.evaluate_expression(&ps.session, &state_script).await;
                event_state = (
                    state
                        .get("defaultPrevented")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    state
                        .get("submitSeen")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
            }

            if !nav_result.did_navigate && meta_href.is_some() && !event_state.0 {
                nav_result = wait_for_explicit_navigation(
                    &ps.session,
                    meta_href.as_deref().unwrap(),
                    &nav_deadline,
                )
                .await
                .unwrap_or(NavigationResult {
                    did_navigate: false,
                    ..Default::default()
                });
            } else if !nav_result.did_navigate && meta_has_submitter && !event_state.1 {
                let submit_script = format!(
                    r#"(() => {{
            const el = document.querySelector({selector_json});
            const submitter = el?.closest?.('button, input[type="submit"], input[type="image"]');
            const form = submitter?.form;
            if (!form || submitter?.type === 'button') return;
            if (typeof form.requestSubmit === 'function') {{
              form.requestSubmit(submitter);
            }} else {{
              form.submit();
            }}
          }})()"#
                );
                self.evaluate_expression::<Value>(&ps.session, &submit_script)
                    .await;
                nav_result = nav_observer
                    .wait(&nav_deadline)
                    .await
                    .unwrap_or(NavigationResult {
                        did_navigate: false,
                        ..Default::default()
                    });
            }
            nav_observer.dispose();

            let title: String = self
                .evaluate_expression(&ps.session, "document.title")
                .await;
            let url: String = self.evaluate_expression(&ps.session, "location.href").await;

            Ok(InternalActionResult {
                page_id: ps.page_id.clone(),
                title,
                url,
                did_navigate: nav_result.did_navigate,
            })
        }
        .await;

        self.dispose_page_session();
        result
    }

    // ── Type ──────────────────────────────────────────────────────────

    async fn r#type(
        &mut self,
        target: &ResolvedTarget,
        text: &str,
        options: TypeOptions,
    ) -> Result<InternalTypeResult, String> {
        let ps = self.active_page_session().await?;
        let result = async {
            install_console_hook(&ps.session).await;

            let timeout_ms = options.timeout_ms.unwrap_or(self.action_timeout_ms);
            let should_clear = options.clear.unwrap_or(true);

            self.wait_for_actionable(&ps.session, &target.selector, &Deadline::new(timeout_ms))
                .await?;

            if should_clear {
                self.focus_and_clear(&ps.session, &target.selector).await?;
            } else {
                let expr = format!(
                    "document.querySelector({})?.focus()",
                    json!(target.selector)
                );
                self.evaluate_expression::<Value>(&ps.session, &expr).await;
            }
            ps.session
                .send(
                    "Input.insertText",
                    Some(&json!({"text": text}).as_object().unwrap().clone()),
                )
                .await
                .map_err(|e| e.to_string())?;

            if options.submit.unwrap_or(false) {
                self.dispatch_enter(&ps.session).await?;
            }

            Ok(InternalTypeResult {
                page_id: ps.page_id.clone(),
                typed: target.selector.clone(),
                submitted: options.submit.unwrap_or(false),
            })
        }
        .await;

        self.dispose_page_session();
        result
    }

    // ── Press ─────────────────────────────────────────────────────────

    async fn press(
        &mut self,
        key: &str,
        target: Option<&ResolvedTarget>,
        _options: PressOptions,
    ) -> Result<InternalActionResult, String> {
        let ps = self.active_page_session().await?;
        let result = async {
            install_console_hook(&ps.session).await;

            let nav_deadline = Deadline::new(self.navigation_timeout_ms);
            let current_loader = self.get_loader_id(&ps.session).await.unwrap_or_default();
            let nav_observer = ActionNavigationObserver::new(&ps.main_frame_id, &current_loader);
            nav_observer.arm(&ps.session);

            if let Some(target) = target {
                self.wait_for_actionable(
                    &ps.session,
                    &target.selector,
                    &Deadline::new(self.action_timeout_ms),
                )
                .await?;
                let expr = format!(
                    "document.querySelector({})?.focus()",
                    json!(target.selector)
                );
                self.evaluate_expression::<Value>(&ps.session, &expr).await;
            }

            let keys = parse_key(key)?;
            for k in &keys {
                with_temporary_preload(&ps.session, async {
                    if k.key == "Enter" && k.r#type == "keyDown" {
                        let mut down = Map::new();
                        down.insert("type".into(), json!("rawKeyDown"));
                        down.insert("key".into(), json!(k.key));
                        down.insert("code".into(), json!(k.code));
                        down.insert(
                            "windowsVirtualKeyCode".into(),
                            json!(k.windows_virtual_key_code),
                        );
                        if k.native_virtual_key_code != 0 {
                            down.insert(
                                "nativeVirtualKeyCode".into(),
                                json!(k.native_virtual_key_code),
                            );
                        }
                        down.insert("modifiers".into(), json!(k.modifiers));
                        ps.session
                            .send("Input.dispatchKeyEvent", Some(&down))
                            .await
                            .map_err(|e| e.to_string())?;

                        let mut char_params = Map::new();
                        char_params.insert("type".into(), json!("char"));
                        char_params.insert("key".into(), json!(k.key));
                        char_params.insert("code".into(), json!(k.code));
                        char_params.insert("text".into(), json!("\r"));
                        char_params.insert(
                            "windowsVirtualKeyCode".into(),
                            json!(k.windows_virtual_key_code),
                        );
                        if k.native_virtual_key_code != 0 {
                            char_params.insert(
                                "nativeVirtualKeyCode".into(),
                                json!(k.native_virtual_key_code),
                            );
                        }
                        char_params.insert("modifiers".into(), json!(k.modifiers));
                        ps.session
                            .send("Input.dispatchKeyEvent", Some(&char_params))
                            .await
                            .map_err(|e| e.to_string())?;
                    } else {
                        let mut params = Map::new();
                        params.insert("type".into(), json!(k.r#type));
                        params.insert("key".into(), json!(k.key));
                        params.insert("code".into(), json!(k.code));
                        if !k.text.is_empty() {
                            params.insert("text".into(), json!(k.text));
                        }
                        params.insert(
                            "windowsVirtualKeyCode".into(),
                            json!(k.windows_virtual_key_code),
                        );
                        if k.native_virtual_key_code != 0 {
                            params.insert(
                                "nativeVirtualKeyCode".into(),
                                json!(k.native_virtual_key_code),
                            );
                        }
                        params.insert("modifiers".into(), json!(k.modifiers));
                        ps.session
                            .send("Input.dispatchKeyEvent", Some(&params))
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })
                .await?;
            }

            let nav_result = nav_observer
                .wait(&nav_deadline)
                .await
                .unwrap_or(NavigationResult {
                    did_navigate: false,
                    ..Default::default()
                });
            nav_observer.dispose();

            let title: String = self
                .evaluate_expression(&ps.session, "document.title")
                .await;
            let url: String = self.evaluate_expression(&ps.session, "location.href").await;

            Ok(InternalActionResult {
                page_id: ps.page_id.clone(),
                title,
                url,
                did_navigate: nav_result.did_navigate,
            })
        }
        .await;

        self.dispose_page_session();
        result
    }

    // ── Tabs ──────────────────────────────────────────────────────────

    async fn tabs(&mut self, action: &TabsAction) -> Result<InternalTabsResult, String> {
        self.init().await?;
        let page_mgr = self.page_mgr.as_mut().unwrap();

        match action {
            TabsAction::List => {
                let pages = page_mgr.get_pages();
                let active_id = page_mgr.get_active_page_id();
                Ok(InternalTabsResult::List {
                    tabs: pages
                        .iter()
                        .enumerate()
                        .map(|(i, p)| InternalTabInfo {
                            page_id: p.target_id.clone(),
                            index: i,
                            title: p.title.clone(),
                            url: p.url.clone(),
                            active: Some(&p.target_id) == active_id.as_ref(),
                        })
                        .collect(),
                })
            }
            TabsAction::New { url } => {
                let (target_id, page) = page_mgr
                    .create_page(url.as_deref().unwrap_or("about:blank"))
                    .await?;
                let pages = page_mgr.get_pages();
                let index = pages
                    .iter()
                    .position(|p| p.target_id == target_id)
                    .unwrap_or(0);
                Ok(InternalTabsResult::New {
                    page: InternalPageInfo {
                        page_id: target_id,
                        title: page.title,
                        url: page.url,
                    },
                    index,
                })
            }
            TabsAction::Select { index } => {
                let pages = page_mgr.get_pages();
                if *index >= pages.len() {
                    return Err(format!("Invalid tab index: {index}"));
                }
                let page = &pages[*index];
                page_mgr.activate_page(&page.target_id).await?;
                Ok(InternalTabsResult::Select {
                    page: InternalPageInfo {
                        page_id: page.target_id.clone(),
                        title: page.title.clone(),
                        url: page.url.clone(),
                    },
                })
            }
            TabsAction::Close { index } => {
                let pages = page_mgr.get_pages();
                if *index >= pages.len() {
                    return Err(format!("Invalid tab index: {index}"));
                }
                let page = &pages[*index];
                let url = page.url.clone();
                page_mgr.close_page(&page.target_id).await?;
                Ok(InternalTabsResult::Close { url, index: *index })
            }
        }
    }

    // ── Evaluate ──────────────────────────────────────────────────────

    async fn evaluate(&mut self, request: &EvaluateRequest) -> Result<Value, String> {
        let ps = self.active_page_session().await?;
        let result = match request {
            EvaluateRequest::Expression { expression } => {
                let params = json!({ "expression": expression, "returnByValue": true });
                let raw = ps
                    .session
                    .send("Runtime.evaluate", params.as_object())
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(raw
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            EvaluateRequest::Function {
                function_declaration,
                arguments,
            } => {
                let args_json: Vec<String> = arguments
                    .iter()
                    .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "null".to_string()))
                    .collect();
                let expression = format!("(({function_declaration})({}))", args_json.join(","));
                let params = json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                });
                let raw = ps
                    .session
                    .send("Runtime.evaluate", params.as_object())
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(raw
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .cloned()
                    .unwrap_or(Value::Null))
            }
        };
        self.dispose_page_session();
        result
    }

    // ── Screenshot ────────────────────────────────────────────────────

    async fn capture_screenshot(
        &mut self,
        options: &CaptureScreenshotOptions,
    ) -> Result<Vec<u8>, String> {
        let ps = self.active_page_session().await?;
        let result = capture_screenshot(&ps.session, options).await;
        self.dispose_page_session();
        result
    }

    // ── Disconnect ────────────────────────────────────────────────────

    async fn disconnect(&mut self) -> Result<(), String> {
        self.dispose_page_session();
        if let Some(connection) = &self.connection {
            connection.disconnect().await;
            self.connection = None;
            self.browser_sess = None;
            self.page_mgr = None;
        }
        Ok(())
    }
}

// ── Module-level helpers ────────────────────────────────────────────

async fn get_main_frame_state(session: &CdpSession) -> Result<(String, String), String> {
    let result = session
        .send("Page.getFrameTree", None)
        .await
        .map_err(|e| e.to_string())?;
    let frame = result.get("frameTree").and_then(|t| t.get("frame"));
    Ok((
        frame
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        frame
            .and_then(|f| f.get("loaderId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::types::DEFAULT_TIMEOUTS;
    use crate::test_cdp::MockCdp;

    /// A ChromiumSession bound to the mock browser.
    fn session_over(mock: &MockCdp) -> ChromiumSession {
        ChromiumSession::new(BrowserSessionParams::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: mock.http_url.clone(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: None,
            init_tab_order: None,
        })
    }

    fn target(selector: &str) -> ResolvedTarget {
        crate::browser::selector::resolve_target(
            Some(selector),
            &crate::browser::types::BrowserConfig::default(),
        )
        .expect("resolve")
    }

    // ── Basics ────────────────────────────────────────────────────────

    #[test]
    fn kind_and_protocol() {
        let mock_ready = ChromiumSession::new(BrowserSessionParams::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: "http://x".to_string(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: None,
            init_tab_order: None,
        });
        assert_eq!(mock_ready.kind(), "chromium");
        assert_eq!(mock_ready.protocol(), "cdp");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn webdriver_params_are_rejected_at_init() {
        let mut s = ChromiumSession::new(BrowserSessionParams::Webdriver {
            endpoint: "http://x".to_string(),
            session_id: "s".to_string(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: None,
        });
        let err = s.open("http://x/", OpenPageOptions::default()).await.unwrap_err();
        assert_eq!(err, "ChromiumSession requires CDP protocol");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn init_failure_propagates() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Target.setDiscoverTargets".to_string());
        let mut s = session_over(&mock);
        let err = s.open("http://x/", OpenPageOptions::default()).await.unwrap_err();
        assert!(err.contains("mock failure"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn frame_tree_failure_surfaces() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Page.getFrameTree".to_string());
        let mut s = session_over(&mock);
        let err = s.open("http://x/", OpenPageOptions::default()).await.unwrap_err();
        assert!(err.contains("mock failure"), "{err}");
    }

    // ── open ──────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn open_navigates_and_reports_page_info() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        let info = s
            .open("http://site/", OpenPageOptions::default())
            .await
            .unwrap();
        assert_eq!(info.title, "Mock Title");
        assert_eq!(info.url, "http://site/");
        assert_eq!(info.page_id, "T-1");
        assert_eq!(mock.commands_of("Page.navigate").len(), 1);
        // Session init sent the enable trio.
        assert_eq!(mock.commands_of("Page.enable").len(), 1);
        assert_eq!(mock.commands_of("Runtime.enable").len(), 1);
        assert_eq!(mock.commands_of("Page.setLifecycleEventsEnabled").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_navigation_error_text_fails() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().navigate_error_text = Some("net::ERR_ABORTED".to_string());
        let mut s = session_over(&mock);
        let err = s
            .open("http://bad/", OpenPageOptions::default())
            .await
            .unwrap_err();
        assert_eq!(err, "Navigation failed: net::ERR_ABORTED");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_same_document_navigation() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().navigate_same_document = true;
        let mut s = session_over(&mock);
        let info = s
            .open("http://x/#frag", OpenPageOptions::default())
            .await
            .unwrap();
        assert_eq!(info.title, "Mock Title");
    }

    // ── click ─────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn click_happy_path_no_navigation() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        let result = s
            .click(&target("#go"), ClickOptions::default())
            .await
            .unwrap();
        assert!(!result.did_navigate);
        assert_eq!(result.title, "Mock Title");
        // mouseMoved + mousePressed + mouseReleased.
        assert_eq!(mock.commands_of("Input.dispatchMouseEvent").len(), 3);
        // Preload script added and removed around the dispatch.
        assert_eq!(mock.commands_of("Page.addScriptToEvaluateOnNewDocument").len(), 1);
        assert_eq!(mock.commands_of("Page.removeScriptToEvaluateOnNewDocument").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_missing_element_times_out_with_not_found() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().element_check = json!({"exists": false});
        let mut s = session_over(&mock);
        let err = s
            .click(&target("#ghost"), ClickOptions { timeout_ms: Some(300) })
            .await
            .unwrap_err();
        assert_eq!(err, "Element not found: \"#ghost\"");
    }

    #[test]
    fn element_check_state_matrix() {
        // Pure state-shaping sanity for the mock (used by click tests).
        let mut state = crate::test_cdp::MockCdpState::default();
        state.element_check = json!({"exists": true, "connected": false, "visible": true, "disabled": false});
        assert_eq!(state.element_check["connected"], json!(false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_not_connected_then_disabled_variants() {
        // connected:false keeps polling until the deadline → not found.
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().element_check =
            json!({"exists": true, "connected": false, "visible": true, "disabled": false});
        let mut s = session_over(&mock);
        let err = s
            .click(&target("#x"), ClickOptions { timeout_ms: Some(250) })
            .await
            .unwrap_err();
        assert!(err.contains("Element not found"), "{err}");

        // visible:false → same polling.
        mock.state.lock().unwrap().element_check =
            json!({"exists": true, "connected": true, "visible": false, "disabled": false});
        let mut s = session_over(&mock);
        let err = s
            .click(&target("#x"), ClickOptions { timeout_ms: Some(250) })
            .await
            .unwrap_err();
        assert!(err.contains("Element not found"), "{err}");

        // disabled → not interactable, no polling.
        mock.state.lock().unwrap().element_check =
            json!({"exists": true, "connected": true, "visible": true, "disabled": true});
        let mut s = session_over(&mock);
        let err = s
            .click(&target("#x"), ClickOptions { timeout_ms: Some(5_000) })
            .await
            .unwrap_err();
        assert_eq!(
            err,
            "Element not interactable: \"#x\" — element is disabled"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_href_fallback_drives_explicit_navigation() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().click_meta =
            json!({"href": "http://fallback/", "hasSubmitter": false});
        let mut s = session_over(&mock);
        let result = s.click(&target("a.link"), ClickOptions::default()).await.unwrap();
        assert!(result.did_navigate);
        // The fallback issued a second Page.navigate to the href.
        let navigations = mock.commands_of("Page.navigate");
        assert_eq!(navigations.len(), 1);
        assert_eq!(navigations[0]["url"], json!("http://fallback/"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_default_prevented_skips_href_fallback() {
        let mock = MockCdp::start().await;
        {
            let mut state = mock.state.lock().unwrap();
            state.click_meta = json!({"href": "http://fallback/", "hasSubmitter": false});
            state.click_state = json!({"defaultPrevented": true, "submitSeen": false});
        }
        let mut s = session_over(&mock);
        let result = s.click(&target("a.link"), ClickOptions::default()).await.unwrap();
        assert!(!result.did_navigate);
        assert!(mock.commands_of("Page.navigate").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_submitter_fallback_runs_request_submit() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().click_meta =
            json!({"href": null, "hasSubmitter": true});
        let mut s = session_over(&mock);
        let result = s
            .click(&target("button.go"), ClickOptions::default())
            .await
            .unwrap();
        assert!(!result.did_navigate);
        // The requestSubmit fallback script ran.
        let evals = mock.commands_of("Runtime.evaluate");
        assert!(
            evals.iter().any(|p| p["expression"]
                .as_str()
                .unwrap_or("")
                .contains("requestSubmit")),
            "evals: {evals:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_submit_seen_skips_request_submit() {
        let mock = MockCdp::start().await;
        {
            let mut state = mock.state.lock().unwrap();
            state.click_meta = json!({"href": null, "hasSubmitter": true});
            state.click_state = json!({"defaultPrevented": false, "submitSeen": true});
        }
        let mut s = session_over(&mock);
        s.click(&target("button.go"), ClickOptions::default())
            .await
            .unwrap();
        let evals = mock.commands_of("Runtime.evaluate");
        assert!(
            !evals.iter().any(|p| p["expression"]
                .as_str()
                .unwrap_or("")
                .contains("requestSubmit"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_observer_detected_navigation() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().navigate_on_input = true;
        let mut s = session_over(&mock);
        let result = s.click(&target("#go"), ClickOptions::default()).await.unwrap();
        assert!(result.did_navigate);
        // Native navigation observed → no state-read, no fallbacks.
        assert!(mock.commands_of("Page.navigate").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_element_check_rpc_failure_keeps_polling() {
        // Runtime.evaluate errors during the check are treated as "not
        // actionable yet" until the deadline.
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Runtime.evaluate".to_string());
        let mut s = session_over(&mock);
        let err = s
            .click(&target("#x"), ClickOptions { timeout_ms: Some(250) })
            .await
            .unwrap_err();
        assert!(err.contains("Element not found"), "{err}");
    }

    // ── type ──────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn type_clear_insert_and_submit_enter() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        let result = s
            .r#type(
                &target("#in"),
                "hello",
                TypeOptions {
                    clear: None,
                    submit: Some(true),
                    timeout_ms: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.typed, "#in");
        assert!(result.submitted);
        assert_eq!(mock.commands_of("Input.insertText").len(), 1);
        // dispatchEnter = rawKeyDown + char + keyUp.
        assert_eq!(mock.commands_of("Input.dispatchKeyEvent").len(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn type_without_clear_or_submit() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        let result = s
            .r#type(
                &target("#in"),
                "x",
                TypeOptions {
                    clear: Some(false),
                    submit: None,
                    timeout_ms: None,
                },
            )
            .await
            .unwrap();
        assert!(!result.submitted);
        assert_eq!(mock.commands_of("Input.insertText").len(), 1);
        assert!(mock.commands_of("Input.dispatchKeyEvent").is_empty());
    }

    // ── press ─────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn press_plain_key_and_enter_combo() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        // Plain key (no target): single keyDown + keyUp pair.
        s.press("a", None, PressOptions::default()).await.unwrap();
        let events = mock.commands_of("Input.dispatchKeyEvent");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], json!("keyDown"));
        assert_eq!(events[1]["type"], json!("keyUp"));

        // Enter with a target: rawKeyDown + char (+ focus eval + keyUp... 
        // Enter path dispatches rawKeyDown+char then the keyUp from the table).
        let mut s = session_over(&mock);
        let before = mock.commands_of("Input.dispatchKeyEvent").len();
        s.press("Enter", Some(&target("#btn")), PressOptions::default())
            .await
            .unwrap();
        let events = mock.commands_of("Input.dispatchKeyEvent");
        let new_events = &events[before..];
        assert!(new_events.iter().any(|e| e["type"] == json!("rawKeyDown")));
        assert!(new_events.iter().any(|e| e["type"] == json!("char")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn press_unknown_key_errors() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        let err = s
            .press("NotARealKey", None, PressOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("Unknown key"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn press_navigation_observed() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().navigate_on_input = true;
        let mut s = session_over(&mock);
        let result = s.press("Enter", None, PressOptions::default()).await.unwrap();
        assert!(result.did_navigate);
    }

    // ── tabs ──────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn tabs_all_actions_against_mock() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);

        // list: one seeded page.
        let result = s.tabs(&TabsAction::List).await.unwrap();
        match result {
            InternalTabsResult::List { tabs } => {
                assert_eq!(tabs.len(), 1);
                assert_eq!(tabs[0].page_id, "T-1");
                assert!(tabs[0].active);
            }
            other => panic!("expected list, got {other:?}"),
        }

        // new.
        let result = s
            .tabs(&TabsAction::New {
                url: Some("http://two/".to_string()),
            })
            .await
            .unwrap();
        match result {
            InternalTabsResult::New { page, index } => {
                assert_eq!(page.url, "http://two/");
                assert_eq!(index, 1);
            }
            other => panic!("expected new, got {other:?}"),
        }

        // select (valid + invalid).
        let result = s.tabs(&TabsAction::Select { index: 0 }).await.unwrap();
        assert!(matches!(result, InternalTabsResult::Select { .. }));
        let err = s.tabs(&TabsAction::Select { index: 99 }).await.unwrap_err();
        assert_eq!(err, "Invalid tab index: 99");

        // close (valid + invalid).
        let result = s.tabs(&TabsAction::Close { index: 1 }).await.unwrap();
        assert!(matches!(result, InternalTabsResult::Close { .. }));
        let err = s.tabs(&TabsAction::Close { index: 99 }).await.unwrap_err();
        assert_eq!(err, "Invalid tab index: 99");
    }

    // ── evaluate ──────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn evaluate_expression_and_function_forms() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        // Expression: location.href via the default responder.
        let v = s
            .evaluate(&EvaluateRequest::Expression {
                expression: "location.href".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(v, json!("about:blank"));
        // Function form (arguments serialized into the IIFE).
        let v = s
            .evaluate(&EvaluateRequest::Function {
                function_declaration: "function(a, b) { return a + b; }".to_string(),
                arguments: vec![json!(1), json!("x")],
            })
            .await
            .unwrap();
        assert_eq!(v, Value::Null);
        let evals = mock.commands_of("Runtime.evaluate");
        let fn_eval = evals
            .iter()
            .find(|p| p["expression"].as_str().unwrap_or("").contains("function(a, b)"))
            .expect("function eval recorded");
        assert!(fn_eval["expression"]
            .as_str()
            .unwrap()
            .contains("1,\"x\""));
        assert_eq!(fn_eval["awaitPromise"], json!(true));

        // Send failure propagates.
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Runtime.evaluate".to_string());
        let mut s = session_over(&mock);
        assert!(s
            .evaluate(&EvaluateRequest::Expression {
                expression: "1".to_string(),
            })
            .await
            .is_err());
    }

    // ── screenshot ────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_screenshot_simple_and_full_page() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        let bytes = s
            .capture_screenshot(&CaptureScreenshotOptions {
                full_page: false,
                format: "png",
                quality: None,
            })
            .await
            .unwrap();
        assert_eq!(bytes, b"\x89PNG-mock".to_vec());

        // Full page: layout metrics feed the clip.
        let mut s = session_over(&mock);
        let bytes = s
            .capture_screenshot(&CaptureScreenshotOptions {
                full_page: true,
                format: "jpeg",
                quality: Some(80),
            })
            .await
            .unwrap();
        assert_eq!(bytes, b"\x89PNG-mock".to_vec());
        let shots = mock.commands_of("Page.captureScreenshot");
        let full = shots.last().unwrap();
        assert_eq!(full["captureBeyondViewport"], json!(true));
        assert_eq!(full["clip"]["width"], json!(800.0));
        assert_eq!(full["quality"], json!(80));
    }

    // ── disconnect ────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnect_is_idempotent() {
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        s.open("http://x/", OpenPageOptions::default()).await.unwrap();
        s.disconnect().await.unwrap();
        // Second disconnect: nothing to tear down → Ok.
        s.disconnect().await.unwrap();
    }

    // ── Remainder coverage ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn second_operation_reuses_initialized_connection() {
        // open() then click(): the second init() call returns early.
        let mock = MockCdp::start().await;
        let mut s = session_over(&mock);
        s.open("http://x/", OpenPageOptions::default()).await.unwrap();
        s.click(&target("#go"), ClickOptions::default()).await.unwrap();
        // One setDiscoverTargets for both operations.
        assert_eq!(mock.commands_of("Target.setDiscoverTargets").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_creates_page_when_browser_has_none() {
        // Zero targets → active_page_session creates about:blank.
        let mock = MockCdp::start_with(vec![], "Chrome/126.0.0.0").await;
        let mut s = session_over(&mock);
        let info = s
            .open("http://fresh/", OpenPageOptions::default())
            .await
            .unwrap();
        assert_eq!(info.url, "http://fresh/");
        assert_eq!(mock.commands_of("Target.createTarget").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn attach_arm_for_pages_without_session() {
        // A page announced ONLY via a targetCreated event (never attached
        // by the page manager) gets attached lazily by active_page_session.
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().events_after_response.insert(
            // After getTargets the targetCreated handler is already
            // registered (setDiscoverTargets would be too early).
            "Target.getTargets".to_string(),
            vec![json!({
                "method": "Target.targetCreated",
                "params": {"targetInfo": {
                    "targetId": "T-late", "type": "page",
                    "url": "about:blank", "title": "",
                }},
            })],
        );
        let mut s = ChromiumSession::new(BrowserSessionParams::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: mock.http_url.clone(),
            timeouts: DEFAULT_TIMEOUTS,
            // Restore the event-announced page as active: it is in the pages
            // map but was never attached.
            active_page_id: Some("T-late".to_string()),
            init_tab_order: None,
        });
        let info = s
            .open("http://late/", OpenPageOptions::default())
            .await
            .unwrap();
        assert_eq!(info.page_id, "T-late");
        // attachToTarget ran for BOTH the seeded page and T-late.
        assert!(mock.commands_of("Target.attachToTarget").len() >= 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn evaluate_failures_default_title_and_url() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Runtime.evaluate".to_string());
        let mut s = session_over(&mock);
        let info = s
            .open("http://x/", OpenPageOptions::default())
            .await
            .unwrap();
        assert_eq!(info.title, "");
        assert_eq!(info.url, "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn click_with_null_scroll_box_clicks_origin() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().eval_override = Some(std::sync::Arc::new(|expr| {
            if expr.contains("scrollIntoView") {
                return Some(Value::Null);
            }
            None
        }));
        let mut s = session_over(&mock);
        s.click(&target("#go"), ClickOptions::default()).await.unwrap();
        let moves = mock.commands_of("Input.dispatchMouseEvent");
        assert_eq!(moves[0]["x"], json!(0));
        assert_eq!(moves[0]["y"], json!(0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn type_clear_focus_failure_propagates() {
        let mock = MockCdp::start().await;
        // The element check passes but the focus/clear eval fails.
        mock.state
            .lock()
            .unwrap()
            .eval_error_on_substring = Some("el.focus".to_string());
        let mut s = session_over(&mock);
        let err = s
            .r#type(
                &target("#in"),
                "x",
                TypeOptions {
                    clear: Some(true),
                    submit: None,
                    timeout_ms: None,
                },
            )
            .await
            .unwrap_err();
        assert!(err.contains("eval failure"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn type_insert_text_failure_propagates() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Input.insertText".to_string());
        let mut s = session_over(&mock);
        let err = s
            .r#type(&target("#in"), "x", TypeOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("mock failure"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn type_submit_enter_dispatch_failure_propagates() {
        let mock = MockCdp::start().await;
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Input.dispatchKeyEvent".to_string());
        let mut s = session_over(&mock);
        let err = s
            .r#type(
                &target("#in"),
                "x",
                TypeOptions {
                    clear: None,
                    submit: Some(true),
                    timeout_ms: None,
                },
            )
            .await
            .unwrap_err();
        assert!(err.contains("mock failure"), "{err}");
    }
}
