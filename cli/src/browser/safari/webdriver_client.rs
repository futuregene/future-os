//! Lightweight W3C WebDriver HTTP client — port of
//! `cli/src/browser/safari/webdriver-client.ts`.
//!
//! Uses reqwest (no npm deps in the TS original). All WebDriver error
//! responses are converted to structured errors preserving HTTP status,
//! error code, message, and stacktrace.

use base64::Engine;
use serde_json::{json, Map, Value};

/// `WebDriverError`.
#[derive(Debug, Clone)]
pub struct WebDriverError {
    pub http_status: u16,
    pub error: String,
    pub message: String,
    pub stacktrace: Option<String>,
}

impl std::fmt::Display for WebDriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WebDriver [{}] {}: {}",
            self.http_status, self.error, self.message
        )
    }
}

/// `WebDriverClient`.
pub struct WebDriverClient {
    base_url: String,
}

impl WebDriverClient {
    pub fn new(base_url: &str) -> Self {
        WebDriverClient {
            base_url: base_url.to_string(),
        }
    }

    // ── Session ────────────────────────────────────────────────────────

    /// `createSession(capabilities?)` — returns the W3C sessionId.
    pub async fn create_session(
        &self,
        capabilities: Option<&Map<String, Value>>,
    ) -> Result<String, String> {
        let caps = capabilities
            .map(|c| Value::Object(c.clone()))
            .unwrap_or_else(|| json!({ "browserName": "safari" }));
        let data = self
            .post(
                "/session",
                Some(&json!({ "capabilities": { "alwaysMatch": caps } })),
            )
            .await?;
        // W3C: sessionId is at root level, but safaridriver puts it under value.
        let sid = data
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                data.get("value")
                    .and_then(Value::as_object)
                    .and_then(|v| v.get("sessionId"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        match sid {
            Some(s) => Ok(s),
            None => Err(format!(
                "No sessionId in createSession response: {}",
                serde_json::to_string(&data).unwrap_or_default()
            )),
        }
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.fetch("DELETE", &format!("/session/{session_id}"), None)
            .await?;
        Ok(())
    }

    // ── Navigation ─────────────────────────────────────────────────────

    pub async fn navigate_to(&self, session_id: &str, url: &str) -> Result<(), String> {
        self.post(
            &format!("/session/{session_id}/url"),
            Some(&json!({ "url": url })),
        )
        .await?;
        Ok(())
    }

    pub async fn get_current_url(&self, session_id: &str) -> Result<String, String> {
        let data = self.get(&format!("/session/{session_id}/url")).await?;
        Ok(data
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub async fn get_title(&self, session_id: &str) -> Result<String, String> {
        let data = self.get(&format!("/session/{session_id}/title")).await?;
        Ok(data
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub async fn get_page_source(&self, session_id: &str) -> Result<String, String> {
        let data = self.get(&format!("/session/{session_id}/source")).await?;
        Ok(data
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    // ── Execute script ─────────────────────────────────────────────────

    pub async fn execute_script<T>(
        &self,
        session_id: &str,
        script: &str,
        args: &[Value],
    ) -> Result<T, String>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let data = self
            .post(
                &format!("/session/{session_id}/execute/sync"),
                Some(&json!({ "script": script, "args": args })),
            )
            .await?;
        let value = data.get("value").cloned().unwrap_or(Value::Null);
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    // ── Elements ───────────────────────────────────────────────────────

    pub async fn find_element(
        &self,
        session_id: &str,
        using: &str,
        value: &str,
    ) -> Result<String, String> {
        let data = self
            .post(
                &format!("/session/{session_id}/element"),
                Some(&json!({ "using": using, "value": value })),
            )
            .await?;
        let element_id = extract_element_id(data.get("value"));
        match element_id {
            Some(id) => Ok(id),
            None => Err("Could not extract element ID from response".to_string()),
        }
    }

    pub async fn find_elements(
        &self,
        session_id: &str,
        using: &str,
        value: &str,
    ) -> Result<Vec<String>, String> {
        let data = self
            .post(
                &format!("/session/{session_id}/elements"),
                Some(&json!({ "using": using, "value": value })),
            )
            .await?;
        let mut out = Vec::new();
        if let Some(items) = data.get("value").and_then(Value::as_array) {
            for item in items {
                if let Some(id) = extract_element_id(Some(item)) {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    pub async fn click_element(&self, session_id: &str, element_id: &str) -> Result<(), String> {
        self.post(
            &format!("/session/{session_id}/element/{element_id}/click"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn get_element_text(
        &self,
        session_id: &str,
        element_id: &str,
    ) -> Result<String, String> {
        let data = self
            .get(&format!("/session/{session_id}/element/{element_id}/text"))
            .await?;
        Ok(data
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub async fn get_element_attribute(
        &self,
        session_id: &str,
        element_id: &str,
        name: &str,
    ) -> Result<Option<String>, String> {
        let data = self
            .get(&format!(
                "/session/{session_id}/element/{element_id}/attribute/{name}"
            ))
            .await?;
        Ok(data
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub async fn send_keys_to_element(
        &self,
        session_id: &str,
        element_id: &str,
        text: &str,
    ) -> Result<(), String> {
        self.post(
            &format!("/session/{session_id}/element/{element_id}/value"),
            Some(&json!({ "text": text })),
        )
        .await?;
        Ok(())
    }

    pub async fn clear_element(&self, session_id: &str, element_id: &str) -> Result<(), String> {
        self.post(
            &format!("/session/{session_id}/element/{element_id}/clear"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn is_element_enabled(
        &self,
        session_id: &str,
        element_id: &str,
    ) -> Result<bool, String> {
        let data = self
            .get(&format!(
                "/session/{session_id}/element/{element_id}/enabled"
            ))
            .await?;
        Ok(data.get("value").and_then(Value::as_bool).unwrap_or(false))
    }

    // ── Screenshot ─────────────────────────────────────────────────────

    pub async fn take_screenshot(&self, session_id: &str) -> Result<Vec<u8>, String> {
        let data = self
            .get(&format!("/session/{session_id}/screenshot"))
            .await?;
        let base64 = data.get("value").and_then(Value::as_str).unwrap_or("");
        base64::engine::general_purpose::STANDARD
            .decode(base64)
            .map_err(|e| format!("Failed to decode screenshot: {e}"))
    }

    // ── Window / tab management ────────────────────────────────────────

    pub async fn get_window_handles(&self, session_id: &str) -> Result<Vec<String>, String> {
        let data = self
            .get(&format!("/session/{session_id}/window/handles"))
            .await?;
        let mut out = Vec::new();
        if let Some(items) = data.get("value").and_then(Value::as_array) {
            for item in items {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                }
            }
        }
        Ok(out)
    }

    pub async fn get_current_window_handle(&self, session_id: &str) -> Result<String, String> {
        let data = self.get(&format!("/session/{session_id}/window")).await?;
        Ok(data
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub async fn switch_to_window(&self, session_id: &str, handle: &str) -> Result<(), String> {
        self.post(
            &format!("/session/{session_id}/window"),
            Some(&json!({ "handle": handle })),
        )
        .await?;
        Ok(())
    }

    pub async fn new_window(&self, session_id: &str) -> Result<String, String> {
        let data = self
            .post(
                &format!("/session/{session_id}/window/new"),
                Some(&json!({ "type": "tab" })),
            )
            .await?;
        let handle = data
            .get("value")
            .and_then(Value::as_object)
            .and_then(|v| v.get("handle"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(handle)
    }

    pub async fn close_window(&self, session_id: &str) -> Result<Vec<String>, String> {
        let data = self
            .fetch("DELETE", &format!("/session/{session_id}/window"), None)
            .await?;
        let mut out = Vec::new();
        if let Some(items) = data.get("value").and_then(Value::as_array) {
            for item in items {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                }
            }
        }
        Ok(out)
    }

    // ── Low-level ──────────────────────────────────────────────────────

    async fn get(&self, path: &str) -> Result<Map<String, Value>, String> {
        self.fetch("GET", path, None).await
    }

    async fn post(&self, path: &str, body: Option<&Value>) -> Result<Map<String, Value>, String> {
        self.fetch("POST", path, body).await
    }

    async fn fetch(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Map<String, Value>, String> {
        let url = format!("{}{}", self.base_url, path);
        let client = reqwest::Client::new();
        let mut req = client.request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
            &url,
        );
        req = req.header("Content-Type", "application/json; charset=utf-8");
        if let Some(body) = body {
            req = req.json(body);
        }

        let response = req.send().await.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| e.to_string())?;

        let data: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                return Err(WebDriverError {
                    http_status: status,
                    error: "invalid response".to_string(),
                    message: text.chars().take(200).collect(),
                    stacktrace: None,
                }
                .to_string());
            }
        };

        // Check for WebDriver error.
        if let Some(val) = data.get("value") {
            if let Some(v) = val.as_object() {
                if let Some(error) = v.get("error").and_then(Value::as_str) {
                    return Err(WebDriverError {
                        http_status: status,
                        error: error.to_string(),
                        message: v
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        stacktrace: v
                            .get("stacktrace")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    }
                    .to_string());
                }
            }
        }

        data.as_object()
            .cloned()
            .ok_or_else(|| "Invalid WebDriver response".to_string())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

const W3C_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

fn extract_element_id(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(obj)) => {
            if let Some(id) = obj.get(W3C_ELEMENT_KEY).and_then(Value::as_str) {
                return Some(id.to_string());
            }
            // Some drivers use "ELEMENT" key (JSON Wire Protocol).
            if let Some(legacy) = obj.get("ELEMENT").and_then(Value::as_str) {
                return Some(legacy.to_string());
            }
            None
        }
        _ => None,
    }
}
