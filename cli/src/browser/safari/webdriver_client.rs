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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{spawn_http, HttpRoute};

    /// Spawn an HTTP mock with the given (path → json body) routes.
    async fn mock(routes: Vec<(&str, u16, &str)>) -> String {
        spawn_http(
            routes
                .into_iter()
                .map(|(p, s, b)| HttpRoute::json(p, s, b))
                .collect(),
        )
        .await
    }

    #[tokio::test]
    async fn webdriver_error_display() {
        let e = WebDriverError {
            http_status: 404,
            error: "no such element".to_string(),
            message: "cannot find".to_string(),
            stacktrace: Some("at x".to_string()),
        };
        assert_eq!(
            format!("{e}"),
            "WebDriver [404] no such element: cannot find"
        );
    }

    #[test]
    fn extract_element_id_shapes() {
        assert_eq!(
            extract_element_id(Some(&json!("plain"))),
            Some("plain".to_string())
        );
        assert_eq!(
            extract_element_id(Some(&json!({"element-6066-11e4-a52e-4f735466cecf": "w3c"}))),
            Some("w3c".to_string())
        );
        assert_eq!(
            extract_element_id(Some(&json!({"ELEMENT": "legacy"}))),
            Some("legacy".to_string())
        );
        assert_eq!(extract_element_id(Some(&json!({"other": 1}))), None);
        assert_eq!(extract_element_id(Some(&json!(42))), None);
        assert_eq!(extract_element_id(None), None);
    }

    #[tokio::test]
    async fn create_session_id_locations_and_missing() {
        // W3C root-level sessionId.
        let base = mock(vec![(
            "/session",
            200,
            r#"{"sessionId":"s-root","value":{}}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        assert_eq!(client.create_session(None).await.unwrap(), "s-root");

        // safaridriver nests it under value.
        let base = mock(vec![(
            "/session",
            200,
            r#"{"value":{"sessionId":"s-nested"}}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        let caps = json!({"browserName": "safari"});
        assert_eq!(
            client
                .create_session(Some(caps.as_object().unwrap()))
                .await
                .unwrap(),
            "s-nested"
        );

        // No sessionId anywhere → descriptive error.
        let base = mock(vec![("/session", 200, r#"{"value":{}}"#)]).await;
        let client = WebDriverClient::new(&base);
        let err = client.create_session(None).await.unwrap_err();
        assert!(
            err.contains("No sessionId in createSession response"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn webdriver_error_response_translation() {
        let base = mock(vec![(
            "/session",
            500,
            r#"{"value":{"error":"session not created","message":"Allow Remote Automation","stacktrace":"trace"}}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        let err = client.create_session(None).await.unwrap_err();
        assert_eq!(
            err,
            "WebDriver [500] session not created: Allow Remote Automation"
        );
    }

    #[tokio::test]
    async fn fetch_invalid_json_and_non_object() {
        // Non-JSON body → "invalid response" with status + truncated text.
        let base = mock(vec![("/session/s1/url", 200, "not json")]).await;
        let client = WebDriverClient::new(&base);
        let err = client.get_current_url("s1").await.unwrap_err();
        assert!(err.contains("invalid response"), "{err}");
        assert!(err.contains("not json"), "{err}");

        // Valid JSON but not an object → "Invalid WebDriver response".
        let base = mock(vec![("/session/s1/url", 200, "[1,2]")]).await;
        let client = WebDriverClient::new(&base);
        let err = client.get_current_url("s1").await.unwrap_err();
        assert_eq!(err, "Invalid WebDriver response");
    }

    #[tokio::test]
    async fn navigation_getters() {
        let base = mock(vec![
            ("/session/s1/url", 200, r#"{"value":"http://x/"}"#),
            ("/session/s1/title", 200, r#"{"value":"Title X"}"#),
            ("/session/s1/source", 200, r#"{"value":"<html/>"}"#),
        ])
        .await;
        let client = WebDriverClient::new(&base);
        client.navigate_to("s1", "http://x/").await.unwrap();
        assert_eq!(client.get_current_url("s1").await.unwrap(), "http://x/");
        assert_eq!(client.get_title("s1").await.unwrap(), "Title X");
        assert_eq!(client.get_page_source("s1").await.unwrap(), "<html/>");
        client.delete_session("s1").await.unwrap();
    }

    #[tokio::test]
    async fn getters_missing_value_default_empty() {
        let base = mock(vec![
            ("/session/s1/url", 200, "{}"),
            ("/session/s1/title", 200, "{}"),
            ("/session/s1/source", 200, "{}"),
        ])
        .await;
        let client = WebDriverClient::new(&base);
        assert_eq!(client.get_current_url("s1").await.unwrap(), "");
        assert_eq!(client.get_title("s1").await.unwrap(), "");
        assert_eq!(client.get_page_source("s1").await.unwrap(), "");
    }

    #[tokio::test]
    async fn execute_script_value_deserialization() {
        let base = mock(vec![(
            "/session/s1/execute/sync",
            200,
            r#"{"value":{"a":1}}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        let v: Value = client.execute_script("s1", "return 1", &[]).await.unwrap();
        assert_eq!(v, json!({"a": 1}));
        // Type mismatch → serde error string.
        let err = client
            .execute_script::<String>("s1", "return 1", &[])
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn find_element_id_shapes() {
        // W3C key.
        let base = mock(vec![(
            "/session/s1/element",
            200,
            r#"{"value":{"element-6066-11e4-a52e-4f735466cecf":"el-1"}}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        assert_eq!(
            client
                .find_element("s1", "css selector", "#a")
                .await
                .unwrap(),
            "el-1"
        );

        // No extractable id.
        let base = mock(vec![("/session/s1/element", 200, r#"{"value":{}}"#)]).await;
        let client = WebDriverClient::new(&base);
        let err = client
            .find_element("s1", "css selector", "#a")
            .await
            .unwrap_err();
        assert_eq!(err, "Could not extract element ID from response");
    }

    #[tokio::test]
    async fn find_elements_collects_known_shapes() {
        let base = mock(vec![(
            "/session/s1/elements",
            200,
            r#"{"value":[
                {"element-6066-11e4-a52e-4f735466cecf":"a"},
                {"ELEMENT":"b"},
                "c",
                {"unknown":"d"},
                42
            ]}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        let ids = client
            .find_elements("s1", "css selector", "div")
            .await
            .unwrap();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn element_interaction_methods() {
        let base = mock(vec![
            ("/session/s1/element/e1/text", 200, r#"{"value":"hello"}"#),
            (
                "/session/s1/element/e1/attribute/href",
                200,
                r#"{"value":"http://l"}"#,
            ),
            (
                "/session/s1/element/e1/attribute/missing",
                200,
                r#"{"value":null}"#,
            ),
            ("/session/s1/element/e1/enabled", 200, r#"{"value":true}"#),
            ("/session/s1/element/e2/enabled", 200, "{}"),
        ])
        .await;
        let client = WebDriverClient::new(&base);
        client.click_element("s1", "e1").await.unwrap();
        assert_eq!(client.get_element_text("s1", "e1").await.unwrap(), "hello");
        assert_eq!(
            client
                .get_element_attribute("s1", "e1", "href")
                .await
                .unwrap(),
            Some("http://l".to_string())
        );
        assert_eq!(
            client
                .get_element_attribute("s1", "e1", "missing")
                .await
                .unwrap(),
            None
        );
        client
            .send_keys_to_element("s1", "e1", "abc")
            .await
            .unwrap();
        client.clear_element("s1", "e1").await.unwrap();
        assert!(client.is_element_enabled("s1", "e1").await.unwrap());
        assert!(!client.is_element_enabled("s1", "e2").await.unwrap());
    }

    #[tokio::test]
    async fn take_screenshot_decodes_base64() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
        let body = format!(r#"{{"value":"{b64}"}}"#);
        let base = mock(vec![("/session/s1/screenshot", 200, &body)]).await;
        let client = WebDriverClient::new(&base);
        assert_eq!(
            client.take_screenshot("s1").await.unwrap(),
            b"png-bytes".to_vec()
        );

        let base = mock(vec![(
            "/session/s1/screenshot",
            200,
            r#"{"value":"!!bad!!"}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        let err = client.take_screenshot("s1").await.unwrap_err();
        assert!(err.contains("Failed to decode screenshot"), "{err}");
    }

    #[tokio::test]
    async fn window_and_tab_management() {
        let base = mock(vec![
            (
                "/session/s1/window/handles",
                200,
                r#"{"value":["h1","h2"]}"#,
            ),
            ("/session/s1/window", 200, r#"{"value":"h1"}"#),
            (
                "/session/s1/window/new",
                200,
                r#"{"value":{"handle":"h3"}}"#,
            ),
        ])
        .await;
        let client = WebDriverClient::new(&base);
        assert_eq!(
            client.get_window_handles("s1").await.unwrap(),
            vec!["h1".to_string(), "h2".to_string()]
        );
        assert_eq!(client.get_current_window_handle("s1").await.unwrap(), "h1");
        client.switch_to_window("s1", "h2").await.unwrap();
        assert_eq!(client.new_window("s1").await.unwrap(), "h3");

        // close_window returns the remaining handles (DELETE).
        let base = mock(vec![("/session/s1/window", 200, r#"{"value":["h2"]}"#)]).await;
        let client = WebDriverClient::new(&base);
        assert_eq!(
            client.close_window("s1").await.unwrap(),
            vec!["h2".to_string()]
        );
    }

    #[tokio::test]
    async fn window_edge_shapes() {
        // Non-array handles → empty.
        let base = mock(vec![(
            "/session/s1/window/handles",
            200,
            r#"{"value":"nope"}"#,
        )])
        .await;
        let client = WebDriverClient::new(&base);
        assert!(client.get_window_handles("s1").await.unwrap().is_empty());

        // Non-array close result → empty.
        let base = mock(vec![("/session/s1/window", 200, r#"{"value":"x"}"#)]).await;
        let client = WebDriverClient::new(&base);
        assert!(client.close_window("s1").await.unwrap().is_empty());

        // new_window without handle → empty string.
        let base = mock(vec![("/session/s1/window/new", 200, r#"{"value":{}}"#)]).await;
        let client = WebDriverClient::new(&base);
        assert_eq!(client.new_window("s1").await.unwrap(), "");
    }

    #[tokio::test]
    async fn connection_failure_is_err() {
        let client = WebDriverClient::new("http://127.0.0.1:1");
        let err = client.get_current_url("s1").await.unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn webdriver_error_arms_propagate_for_every_method() {
        // Every endpoint answers with a WebDriver error payload → every
        // method surfaces it (covers the per-method error arms).
        const ERR: &str = r#"{"value":{"error":"unknown error","message":"kaput"}}"#;
        let base = mock(vec![
            ("/session", 500, ERR),
            ("/session/s1", 500, ERR),
            ("/session/s1/url", 500, ERR),
            ("/session/s1/title", 500, ERR),
            ("/session/s1/source", 500, ERR),
            ("/session/s1/execute/sync", 500, ERR),
            ("/session/s1/element", 500, ERR),
            ("/session/s1/elements", 500, ERR),
            ("/session/s1/element/e1/click", 500, ERR),
            ("/session/s1/element/e1/text", 500, ERR),
            ("/session/s1/element/e1/attribute/href", 500, ERR),
            ("/session/s1/element/e1/value", 500, ERR),
            ("/session/s1/element/e1/clear", 500, ERR),
            ("/session/s1/element/e1/enabled", 500, ERR),
            ("/session/s1/screenshot", 500, ERR),
            ("/session/s1/window/handles", 500, ERR),
            ("/session/s1/window", 500, ERR),
            ("/session/s1/window/new", 500, ERR),
        ])
        .await;
        let client = WebDriverClient::new(&base);

        macro_rules! assert_kaput {
            ($call:expr) => {
                let err = $call.await.unwrap_err();
                assert!(err.contains("kaput"), "{err}");
            };
        }
        assert_kaput!(client.delete_session("s1"));
        assert_kaput!(client.navigate_to("s1", "http://x/"));
        assert_kaput!(client.get_current_url("s1"));
        assert_kaput!(client.get_title("s1"));
        assert_kaput!(client.get_page_source("s1"));
        assert_kaput!(client.execute_script::<Value>("s1", "return 1", &[]));
        assert_kaput!(client.find_element("s1", "css selector", "#a"));
        assert_kaput!(client.find_elements("s1", "css selector", "#a"));
        assert_kaput!(client.click_element("s1", "e1"));
        assert_kaput!(client.get_element_text("s1", "e1"));
        assert_kaput!(client.get_element_attribute("s1", "e1", "href"));
        assert_kaput!(client.send_keys_to_element("s1", "e1", "t"));
        assert_kaput!(client.clear_element("s1", "e1"));
        assert_kaput!(client.is_element_enabled("s1", "e1"));
        assert_kaput!(client.take_screenshot("s1"));
        assert_kaput!(client.get_window_handles("s1"));
        assert_kaput!(client.get_current_window_handle("s1"));
        assert_kaput!(client.switch_to_window("s1", "h1"));
        assert_kaput!(client.new_window("s1"));
        assert_kaput!(client.close_window("s1"));
        // create_session surfaces the error too.
        assert_kaput!(client.create_session(None));
    }

    #[test]
    fn extract_element_id_object_without_known_keys() {
        assert_eq!(extract_element_id(Some(&json!({"bogus": true}))), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_array_values_yield_empty_lists() {
        let base = spawn_http(vec![
            HttpRoute::json(
                "/session/s1/elements",
                200,
                r#"{"value": {"not": "array"}}"#,
            ),
            HttpRoute::json("/session/s1/window/handles", 200, r#"{"value": null}"#),
            HttpRoute::json("/session/s1/window", 200, r#"{"value": "h1"}"#),
        ])
        .await;
        let client = WebDriverClient::new(&base);
        assert!(client
            .find_elements("s1", "css selector", ".x")
            .await
            .unwrap()
            .is_empty());
        assert!(client.get_window_handles("s1").await.unwrap().is_empty());
        // close_window parses the same shape; a non-array value → empty.
        assert!(client.close_window("s1").await.unwrap().is_empty());
    }
}
