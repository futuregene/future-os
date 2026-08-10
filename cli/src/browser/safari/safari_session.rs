//! SafariSession — BrowserSession implementation via W3C WebDriver protocol.
//! Port of `cli/src/browser/safari/safari-session.ts`.
//!
//! Capability gaps vs Chromium CDP:
//! - No fullPage screenshot (WebDriver limitation)
//! - WebDriver Element Click instead of Input.dispatchMouseEvent
//! - execute/sync for console hook

use super::webdriver_client::WebDriverClient;
use crate::browser::backend::BrowserSessionParams;
use crate::browser::backend::{
    BrowserSession, CaptureScreenshotOptions, ClickOptions, Deadline, EvaluateRequest,
    InternalActionResult, InternalPageInfo, InternalTabInfo, InternalTabsResult,
    InternalTypeResult, OpenPageOptions, PressOptions, ResolvedTarget, TabsAction, TypeOptions,
};
use crate::browser::errors::{element_not_found_error, unsupported_capability_error};
use crate::browser::scripts::console_hook_invocation_source;
use async_trait::async_trait;
use serde_json::Value;

/// `SafariSession`.
pub struct SafariSession {
    client: WebDriverClient,
    session_id: String,
}

impl SafariSession {
    pub fn new(params: BrowserSessionParams) -> Result<Self, String> {
        if params.protocol() != "webdriver" {
            return Err("SafariSession requires webdriver protocol".to_string());
        }
        let session_id = match &params {
            BrowserSessionParams::Webdriver { session_id, .. } => session_id.clone(),
            _ => unreachable!("protocol checked above"),
        };
        Ok(SafariSession {
            client: WebDriverClient::new(params.endpoint()),
            session_id,
        })
    }

    /// Resolve a CSS selector to a WebDriver element ID; xpath=/text= prefixes
    /// are translated (text= → xpath contains()).
    async fn find_one(&self, selector: &str) -> Result<String, String> {
        let mut using = "css selector";
        let mut value: String = selector.to_string();

        if let Some(body) = selector.strip_prefix("xpath=") {
            using = "xpath";
            value = body.to_string();
        } else if let Some(text) = selector.strip_prefix("text=") {
            using = "xpath";
            value = format!("//*[contains(text(),\"{text}\")]");
        }

        match self
            .client
            .find_element(&self.session_id, using, &value)
            .await
        {
            Ok(id) => Ok(id),
            Err(e) => {
                // WebDriver "no such element" → ElementNotFoundError.
                if let Some(wd) = parse_webdriver_error(&e) {
                    if wd.error == "no such element" {
                        return Err(element_not_found_error(selector).to_string());
                    }
                }
                Err(e)
            }
        }
    }

    async fn current_page_id(&self) -> Result<String, String> {
        let handle = self
            .client
            .get_current_window_handle(&self.session_id)
            .await?;
        Ok(handle)
    }

    async fn install_console_hook(&self) {
        let _ = self
            .client
            .execute_script::<Value>(&self.session_id, &console_hook_invocation_source(), &[])
            .await;
    }
}

/// Extract a WebDriver error payload from a formatted error string. The
/// client formats `WebDriver [status] code: message`, so we re-parse the
/// "no such element" marker here (the TS checks `e.wd.error` directly).
fn parse_webdriver_error(e: &str) -> Option<WebDriverErrMarker> {
    let marker = e.find("no such element")?;
    Some(WebDriverErrMarker {
        error: e[marker..marker + "no such element".len()].to_string(),
    })
}

struct WebDriverErrMarker {
    error: String,
}

#[async_trait]
impl BrowserSession for SafariSession {
    fn kind(&self) -> &'static str {
        "safari"
    }

    fn protocol(&self) -> &'static str {
        "webdriver"
    }

    async fn open(
        &mut self,
        url: &str,
        _options: OpenPageOptions,
    ) -> Result<InternalPageInfo, String> {
        self.client.navigate_to(&self.session_id, url).await?;

        // Wait for page to load (WebDriver navigateTo waits for page load).
        let handle = self
            .client
            .get_current_window_handle(&self.session_id)
            .await?;
        let page_id = handle;

        // Install console hook.
        self.install_console_hook().await;

        let title = self.client.get_title(&self.session_id).await?;
        let current_url = self.client.get_current_url(&self.session_id).await?;

        Ok(InternalPageInfo {
            page_id,
            title,
            url: current_url,
        })
    }

    async fn click(
        &mut self,
        target: &ResolvedTarget,
        _options: ClickOptions,
    ) -> Result<InternalActionResult, String> {
        let element_id = self.find_one(&target.selector).await?;

        let handle = self
            .client
            .get_current_window_handle(&self.session_id)
            .await?;
        let current_url = self.client.get_current_url(&self.session_id).await?;

        self.client
            .click_element(&self.session_id, &element_id)
            .await?;

        // Check if navigation happened (short window — no point waiting 15s).
        let nav_deadline = Deadline::new(500);
        let mut new_url = current_url.clone();
        while !nav_deadline.expired() {
            new_url = self.client.get_current_url(&self.session_id).await?;
            if new_url != current_url {
                break;
            }
            crate::utils::time::sleep(100).await;
        }

        let title = self.client.get_title(&self.session_id).await?;

        Ok(InternalActionResult {
            page_id: handle,
            title,
            url: new_url.clone(),
            did_navigate: new_url != current_url,
        })
    }

    async fn r#type(
        &mut self,
        target: &ResolvedTarget,
        text: &str,
        options: TypeOptions,
    ) -> Result<InternalTypeResult, String> {
        let element_id = self.find_one(&target.selector).await?;

        let should_clear = options.clear.unwrap_or(true);
        if should_clear {
            self.client
                .clear_element(&self.session_id, &element_id)
                .await?;
        }
        self.client
            .send_keys_to_element(&self.session_id, &element_id, text)
            .await?;

        if options.submit.unwrap_or(false) {
            self.client
                .send_keys_to_element(&self.session_id, &element_id, "\n")
                .await?;
        }

        Ok(InternalTypeResult {
            page_id: self.current_page_id().await?,
            typed: target.selector.clone(),
            submitted: options.submit.unwrap_or(false),
        })
    }

    async fn press(
        &mut self,
        key: &str,
        target: Option<&ResolvedTarget>,
        _options: PressOptions,
    ) -> Result<InternalActionResult, String> {
        // Map common keys to WebDriver sendKeys sequences.
        let key_map: &[(&str, &str)] = &[
            ("Enter", "\u{E007}"),
            ("Tab", "\u{E004}"),
            ("Escape", "\u{E00C}"),
            ("Backspace", "\u{E003}"),
            ("Delete", "\u{E017}"),
            ("Space", " "),
            ("ArrowUp", "\u{E013}"),
            ("ArrowDown", "\u{E015}"),
            ("ArrowLeft", "\u{E012}"),
            ("ArrowRight", "\u{E014}"),
            ("Home", "\u{E011}"),
            ("End", "\u{E010}"),
            ("PageUp", "\u{E00E}"),
            ("PageDown", "\u{E00F}"),
        ];
        let webdriver_key = key_map
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .unwrap_or(key);

        if let Some(target) = target {
            let element_id = self.find_one(&target.selector).await?;
            self.client
                .send_keys_to_element(&self.session_id, &element_id, webdriver_key)
                .await?;
        } else {
            // Send key to the active element.
            let element_id = self
                .client
                .find_element(&self.session_id, "css selector", "body")
                .await?;
            self.client
                .send_keys_to_element(&self.session_id, &element_id, webdriver_key)
                .await?;
        }

        let handle = self
            .client
            .get_current_window_handle(&self.session_id)
            .await?;
        let title = self.client.get_title(&self.session_id).await?;
        let url = self.client.get_current_url(&self.session_id).await?;

        Ok(InternalActionResult {
            page_id: handle,
            title,
            url,
            did_navigate: false,
        })
    }

    async fn tabs(&mut self, action: &TabsAction) -> Result<InternalTabsResult, String> {
        if let TabsAction::List = action {
            let handles = self.client.get_window_handles(&self.session_id).await?;
            let current_handle = self
                .client
                .get_current_window_handle(&self.session_id)
                .await?;
            let mut tabs = Vec::new();

            for (i, handle) in handles.iter().enumerate() {
                // Switch to each window to get title (expensive but needed).
                let _ = self.client.switch_to_window(&self.session_id, handle).await;
                let title = self
                    .client
                    .get_title(&self.session_id)
                    .await
                    .unwrap_or_default();
                let url = self
                    .client
                    .get_current_url(&self.session_id)
                    .await
                    .unwrap_or_default();

                tabs.push(InternalTabInfo {
                    page_id: handle.clone(),
                    index: i,
                    title,
                    url,
                    active: *handle == current_handle,
                });
            }

            // Switch back to original.
            if !handles.is_empty() {
                let _ = self
                    .client
                    .switch_to_window(&self.session_id, &current_handle)
                    .await;
            }

            return Ok(InternalTabsResult::List { tabs });
        }

        if let TabsAction::New { url } = action {
            let handle = self.client.new_window(&self.session_id).await?;
            if let Some(url) = url {
                self.client.navigate_to(&self.session_id, url).await?;
            }

            // Install console hook on new page.
            self.install_console_hook().await;

            let handles = self.client.get_window_handles(&self.session_id).await?;
            let index = handles.iter().position(|h| h == &handle).unwrap_or(0);

            let title = self
                .client
                .get_title(&self.session_id)
                .await
                .unwrap_or_default();
            let url = self.client.get_current_url(&self.session_id).await?;

            return Ok(InternalTabsResult::New {
                page: InternalPageInfo {
                    page_id: handle,
                    title,
                    url,
                },
                index,
            });
        }

        let handles = self.client.get_window_handles(&self.session_id).await?;
        let (index, is_select, is_close) = match action {
            TabsAction::Select { index } => (*index, true, false),
            TabsAction::Close { index } => (*index, false, true),
            TabsAction::List | TabsAction::New { .. } => unreachable!("handled above"),
        };
        if index >= handles.len() {
            return Err(format!("Invalid tab index: {index}"));
        }

        if is_select {
            let handle = &handles[index];
            self.client
                .switch_to_window(&self.session_id, handle)
                .await?;

            // Reinstall console hook on newly-focused window.
            self.install_console_hook().await;

            let title = self
                .client
                .get_title(&self.session_id)
                .await
                .unwrap_or_default();
            let url = self.client.get_current_url(&self.session_id).await?;

            return Ok(InternalTabsResult::Select {
                page: InternalPageInfo {
                    page_id: handle.clone(),
                    title,
                    url,
                },
            });
        }

        if is_close {
            let handle = &handles[index];
            // Switch to the window first, then close it.
            self.client
                .switch_to_window(&self.session_id, handle)
                .await?;
            let url = self
                .client
                .get_current_url(&self.session_id)
                .await
                .unwrap_or_default();
            self.client.close_window(&self.session_id).await?;

            // After closing, switch to the last remaining window.
            let remaining = self.client.get_window_handles(&self.session_id).await?;
            if !remaining.is_empty() {
                let _ = self
                    .client
                    .switch_to_window(&self.session_id, remaining.last().unwrap())
                    .await;
            }

            return Ok(InternalTabsResult::Close { url, index });
        }

        unreachable!("tabs action exhaustive")
    }

    async fn evaluate(&mut self, request: &EvaluateRequest) -> Result<Value, String> {
        if let EvaluateRequest::Expression { expression } = request {
            let script = format!("return ({expression})");
            return self
                .client
                .execute_script::<Value>(&self.session_id, &script, &[])
                .await;
        }

        // Function call: wrap as IIFE.
        if let EvaluateRequest::Function {
            function_declaration,
            arguments,
        } = request
        {
            let expr = format!("return ({function_declaration}).apply(null, arguments);");
            return self
                .client
                .execute_script::<Value>(&self.session_id, &expr, arguments)
                .await;
        }
        unreachable!("evaluate request exhaustive")
    }

    async fn capture_screenshot(
        &mut self,
        options: &CaptureScreenshotOptions,
    ) -> Result<Vec<u8>, String> {
        if options.full_page {
            return Err(unsupported_capability_error(
                "safari",
                "Full-page screenshot",
                Some("Use viewport screenshot or a Chrome/Edge browser."),
            )
            .to_string());
        }
        self.client.take_screenshot(&self.session_id).await
    }

    async fn disconnect(&mut self) -> Result<(), String> {
        // Don't delete the session — it persists across CLI commands.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::types::DEFAULT_TIMEOUTS;
    use crate::test_server::{spawn_http, HttpRoute};
    use serde_json::json;

    fn session(endpoint: &str) -> SafariSession {
        SafariSession::new(BrowserSessionParams::Webdriver {
            endpoint: endpoint.to_string(),
            session_id: "s1".to_string(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: None,
        })
        .expect("session")
    }

    fn target(selector: &str) -> ResolvedTarget {
        crate::browser::selector::resolve_target(
            Some(selector),
            &crate::browser::types::BrowserConfig::default(),
        )
        .expect("resolve")
    }

    async fn mock(routes: Vec<HttpRoute>) -> String {
        spawn_http(routes).await
    }

    #[tokio::test]
    async fn new_rejects_cdp_params() {
        let err = SafariSession::new(BrowserSessionParams::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: "http://x".to_string(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: None,
            init_tab_order: None,
        })
        .err()
        .expect("cdp params must fail");
        assert_eq!(err, "SafariSession requires webdriver protocol");
    }

    #[tokio::test]
    async fn kind_and_protocol() {
        let s = session("http://127.0.0.1:1");
        assert_eq!(s.kind(), "safari");
        assert_eq!(s.protocol(), "webdriver");
    }

    #[tokio::test]
    async fn open_navigates_and_reports_title_url() {
        let base = mock(vec![
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"http://page/"}"#),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":"h1"}"#),
            HttpRoute::json("/session/s1/title", 200, r#"{"value":"Page T"}"#),
            HttpRoute::json("/session/s1/execute/sync", 200, r#"{"value":null}"#),
        ])
        .await;
        let mut s = session(&base);
        let info = s
            .open("http://page/", OpenPageOptions::default())
            .await
            .unwrap();
        assert_eq!(info.page_id, "h1");
        assert_eq!(info.title, "Page T");
        assert_eq!(info.url, "http://page/");
    }

    #[tokio::test]
    async fn click_detects_navigation_via_url_change() {
        let base = mock(vec![
            HttpRoute::json("/session/s1/element", 200, r#"{"value":"e1"}"#),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":"h1"}"#),
            HttpRoute::sequence(
                "/session/s1/url",
                vec![
                    (200, r#"{"value":"http://old/"}"#),
                    (200, r#"{"value":"http://new/"}"#),
                ],
            ),
            HttpRoute::json("/session/s1/element/e1/click", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/title", 200, r#"{"value":"New T"}"#),
        ])
        .await;
        let mut s = session(&base);
        let result = s
            .click(&target("#go"), ClickOptions::default())
            .await
            .unwrap();
        assert!(result.did_navigate);
        assert_eq!(result.url, "http://new/");
        assert_eq!(result.title, "New T");
        assert_eq!(result.page_id, "h1");
    }

    #[tokio::test]
    async fn click_without_navigation() {
        let base = mock(vec![
            HttpRoute::json("/session/s1/element", 200, r#"{"value":"e1"}"#),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":"h1"}"#),
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"http://same/"}"#),
            HttpRoute::json("/session/s1/element/e1/click", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/title", 200, r#"{"value":"Same"}"#),
        ])
        .await;
        let mut s = session(&base);
        let result = s
            .click(&target("#stay"), ClickOptions::default())
            .await
            .unwrap();
        assert!(!result.did_navigate);
        assert_eq!(result.url, "http://same/");
    }

    #[tokio::test]
    async fn click_missing_element_maps_to_not_found() {
        let base = mock(vec![HttpRoute::json(
            "/session/s1/element",
            404,
            r#"{"value":{"error":"no such element","message":"nope"}}"#,
        )])
        .await;
        let mut s = session(&base);
        let err = s
            .click(&target("#ghost"), ClickOptions::default())
            .await
            .unwrap_err();
        assert_eq!(err, "Element not found: \"#ghost\"");
    }

    #[tokio::test]
    async fn find_one_translates_xpath_and_text_prefixes() {
        let base = mock(vec![HttpRoute::json(
            "/session/s1/element",
            200,
            r#"{"value":"e9"}"#,
        )])
        .await;
        let s = session(&base);
        assert_eq!(s.find_one("xpath=//div").await.unwrap(), "e9");
        assert_eq!(s.find_one("text=Hello").await.unwrap(), "e9");
        assert_eq!(s.find_one("#css").await.unwrap(), "e9");
    }

    #[tokio::test]
    async fn find_one_non_element_error_passthrough() {
        let base = mock(vec![HttpRoute::json(
            "/session/s1/element",
            500,
            r#"{"value":{"error":"unknown error","message":"driver exploded"}}"#,
        )])
        .await;
        let s = session(&base);
        let err = s.find_one("#x").await.unwrap_err();
        assert!(err.contains("driver exploded"), "{err}");
    }

    #[tokio::test]
    async fn type_clear_and_submit_sequences() {
        let base = mock(vec![
            HttpRoute::json("/session/s1/element", 200, r#"{"value":"e1"}"#),
            HttpRoute::json("/session/s1/element/e1/clear", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/element/e1/value", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":"h1"}"#),
        ])
        .await;
        let mut s = session(&base);
        // clear (default) + submit.
        let r = s
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
        assert_eq!(r.typed, "#in");
        assert!(r.submitted);
        assert_eq!(r.page_id, "h1");

        // clear: false, submit: false.
        let r = s
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
        assert!(!r.submitted);
    }

    #[tokio::test]
    async fn press_maps_keys_and_sends_to_body_without_target() {
        let base = mock(vec![
            HttpRoute::json("/session/s1/element", 200, r#"{"value":"e-body"}"#),
            HttpRoute::json("/session/s1/element/e-body/value", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":"h1"}"#),
            HttpRoute::json("/session/s1/title", 200, r#"{"value":"T"}"#),
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"http://u/"}"#),
        ])
        .await;
        let mut s = session(&base);
        // Mapped key without a target → sent to body element.
        let r = s
            .press("Enter", None, PressOptions::default())
            .await
            .unwrap();
        assert!(!r.did_navigate);
        assert_eq!(r.page_id, "h1");
        // Unmapped key passes through verbatim; with target.
        let r = s
            .press("F5", Some(&target("#btn")), PressOptions::default())
            .await
            .unwrap();
        assert_eq!(r.url, "http://u/");
    }

    #[tokio::test]
    async fn tabs_list_switches_through_handles() {
        let base = mock(vec![
            HttpRoute::json(
                "/session/s1/window/handles",
                200,
                r#"{"value":["h1","h2"]}"#,
            ),
            HttpRoute::sequence(
                "/session/s1/window",
                vec![
                    (200, r#"{"value":"h1"}"#), // current handle
                    (200, r#"{"value":null}"#), // switch h1
                    (200, r#"{"value":null}"#), // switch h2
                    (200, r#"{"value":null}"#), // switch back
                ],
            ),
            HttpRoute::sequence(
                "/session/s1/title",
                vec![(200, r#"{"value":"T1"}"#), (200, r#"{"value":"T2"}"#)],
            ),
            HttpRoute::sequence(
                "/session/s1/url",
                vec![(200, r#"{"value":"u1"}"#), (200, r#"{"value":"u2"}"#)],
            ),
        ])
        .await;
        let mut s = session(&base);
        let result = s.tabs(&TabsAction::List).await.unwrap();
        match result {
            InternalTabsResult::List { tabs } => {
                assert_eq!(tabs.len(), 2);
                assert_eq!(tabs[0].title, "T1");
                assert!(tabs[0].active);
                assert_eq!(tabs[1].title, "T2");
                assert!(!tabs[1].active);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tabs_new_with_and_without_url() {
        let base = mock(vec![
            HttpRoute::json(
                "/session/s1/window/new",
                200,
                r#"{"value":{"handle":"h2"}}"#,
            ),
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"http://new/"}"#),
            HttpRoute::json("/session/s1/execute/sync", 200, r#"{"value":null}"#),
            HttpRoute::json(
                "/session/s1/window/handles",
                200,
                r#"{"value":["h1","h2"]}"#,
            ),
            HttpRoute::json("/session/s1/title", 200, r#"{"value":"NT"}"#),
        ])
        .await;
        let mut s = session(&base);
        let result = s
            .tabs(&TabsAction::New {
                url: Some("http://new/".to_string()),
            })
            .await
            .unwrap();
        match result {
            InternalTabsResult::New { page, index } => {
                assert_eq!(page.page_id, "h2");
                assert_eq!(index, 1);
                assert_eq!(page.url, "http://new/");
            }
            other => panic!("expected new, got {other:?}"),
        }
        // No url → skip navigate.
        let result = s.tabs(&TabsAction::New { url: None }).await.unwrap();
        assert!(matches!(result, InternalTabsResult::New { .. }));
    }

    #[tokio::test]
    async fn tabs_select_and_invalid_index() {
        let base = mock(vec![
            HttpRoute::json(
                "/session/s1/window/handles",
                200,
                r#"{"value":["h1","h2"]}"#,
            ),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/execute/sync", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/title", 200, r#"{"value":"T2"}"#),
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"u2"}"#),
        ])
        .await;
        let mut s = session(&base);
        let result = s.tabs(&TabsAction::Select { index: 1 }).await.unwrap();
        match result {
            InternalTabsResult::Select { page } => {
                assert_eq!(page.page_id, "h2");
                assert_eq!(page.title, "T2");
            }
            other => panic!("expected select, got {other:?}"),
        }
        let err = s.tabs(&TabsAction::Select { index: 9 }).await.unwrap_err();
        assert_eq!(err, "Invalid tab index: 9");
    }

    #[tokio::test]
    async fn tabs_close_switches_back_to_last_remaining() {
        let base = mock(vec![
            HttpRoute::json(
                "/session/s1/window/handles",
                200,
                r#"{"value":["h1","h2"]}"#,
            ),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"u1"}"#),
        ])
        .await;
        let mut s = session(&base);
        let result = s.tabs(&TabsAction::Close { index: 0 }).await.unwrap();
        match result {
            InternalTabsResult::Close { url, index } => {
                assert_eq!(url, "u1");
                assert_eq!(index, 0);
            }
            other => panic!("expected close, got {other:?}"),
        }
        // Close with an empty remaining list.
        let base = mock(vec![
            HttpRoute::sequence(
                "/session/s1/window/handles",
                vec![(200, r#"{"value":["h1"]}"#), (200, r#"{"value":[]}"#)],
            ),
            HttpRoute::json("/session/s1/window", 200, r#"{"value":null}"#),
            HttpRoute::json("/session/s1/url", 200, r#"{"value":"u1"}"#),
        ])
        .await;
        let mut s = session(&base);
        let result = s.tabs(&TabsAction::Close { index: 0 }).await.unwrap();
        assert!(matches!(result, InternalTabsResult::Close { .. }));
    }

    #[tokio::test]
    async fn evaluate_expression_and_function() {
        let base = mock(vec![HttpRoute::json(
            "/session/s1/execute/sync",
            200,
            r#"{"value":{"k":2}}"#,
        )])
        .await;
        let mut s = session(&base);
        let v = s
            .evaluate(&EvaluateRequest::Expression {
                expression: "1+1".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(v, json!({"k": 2}));
        let v = s
            .evaluate(&EvaluateRequest::Function {
                function_declaration: "function(a){return a;}".to_string(),
                arguments: vec![json!(1)],
            })
            .await
            .unwrap();
        assert_eq!(v, json!({"k": 2}));
    }

    #[tokio::test]
    async fn capture_screenshot_full_page_unsupported_and_viewport_ok() {
        use base64::Engine;
        let mut s = session("http://127.0.0.1:1");
        let err = s
            .capture_screenshot(&CaptureScreenshotOptions {
                full_page: true,
                format: "png",
                quality: None,
            })
            .await
            .unwrap_err();
        assert!(
            err.contains("Full-page screenshot is not supported on safari"),
            "{err}"
        );
        assert!(
            err.contains("Use viewport screenshot or a Chrome/Edge browser."),
            "{err}"
        );

        let b64 = base64::engine::general_purpose::STANDARD.encode(b"img");
        let base = mock(vec![HttpRoute::json(
            "/session/s1/screenshot",
            200,
            &format!(r#"{{"value":"{b64}"}}"#),
        )])
        .await;
        let mut s = session(&base);
        let bytes = s
            .capture_screenshot(&CaptureScreenshotOptions {
                full_page: false,
                format: "png",
                quality: None,
            })
            .await
            .unwrap();
        assert_eq!(bytes, b"img".to_vec());
    }

    #[tokio::test]
    async fn disconnect_is_ok() {
        let mut s = session("http://127.0.0.1:1");
        s.disconnect().await.unwrap();
    }
}
