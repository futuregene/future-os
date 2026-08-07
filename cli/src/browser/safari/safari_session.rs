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
