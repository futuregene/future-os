//! Browser pipeline state persistence — port of
//! `cli/src/browser/browser-state.ts`.
//!
//! Reads/writes `~/.future/agent/browser/config.json` (honors `FUTURE_HOME`),
//! handles v1 → v2 migration and runtime validation.

use crate::browser::errors::{invalid_browser_config_error, BrowserError};
use crate::browser::types::{BrowserConfig, BrowserConnectionConfig, CURRENT_CONFIG_VERSION};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// `~/.future/agent/browser` (honors `FUTURE_HOME`).
pub fn browser_dir() -> PathBuf {
    let future_home = std::env::var("FUTURE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".future"));
    future_home.join("agent").join("browser")
}

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:9222";

/// `loadBrowserConfig()` — ENOENT → default.
pub async fn load_browser_config() -> Result<BrowserConfig, BrowserError> {
    let config_file = browser_dir().join("config.json");
    match tokio::fs::read_to_string(&config_file).await {
        Ok(raw) => {
            let parsed: Value = serde_json::from_str(&raw)
                .map_err(|_| invalid_browser_config_error("Invalid JSON in browser config file"))?;
            parse_browser_config(&parsed)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default_browser_config()),
        Err(e) => Err(invalid_browser_config_error(format!("{e}"))),
    }
}

/// `saveBrowserConfig(config)`.
pub async fn save_browser_config(config: &BrowserConfig) -> Result<(), BrowserError> {
    tokio::fs::create_dir_all(browser_dir())
        .await
        .map_err(|e| invalid_browser_config_error(format!("{e}")))?;
    let value = config_to_json(config);
    // Invariant: config_to_json produces plain JSON values, which always
    // serialize — no error arm to cover.
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("config json serializes")
    );
    tokio::fs::write(browser_dir().join("config.json"), text)
        .await
        .map_err(|e| invalid_browser_config_error(format!("{e}")))
}

/// `defaultBrowserConfig()`.
pub fn default_browser_config() -> BrowserConfig {
    BrowserConfig {
        version: CURRENT_CONFIG_VERSION,
        connection: BrowserConnectionConfig::Cdp {
            browser_kind: "chromium".to_string(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
        },
        ..Default::default()
    }
}

/// `parseBrowserConfig(raw)` — migration + validation. Mirrors the exact TS
/// error messages (the CLI surface prints them verbatim).
pub fn parse_browser_config(raw: &Value) -> Result<BrowserConfig, BrowserError> {
    let Some(obj) = raw.as_object() else {
        return Err(invalid_browser_config_error(
            "Browser config must be a JSON object",
        ));
    };

    let version = obj.get("version");

    // Missing version or 1 → migrate
    if version.is_none() || version == Some(&Value::Number(1.into())) {
        return migrate_v1_config(obj);
    }

    // Unknown future version
    if let Some(Value::Number(n)) = version {
        if let Some(v) = n.as_i64() {
            if v > CURRENT_CONFIG_VERSION {
                return Err(invalid_browser_config_error(format!(
                    "Unsupported browser config version: {v}. Expected ≤ {CURRENT_CONFIG_VERSION}."
                )));
            }
            if v == CURRENT_CONFIG_VERSION {
                return validate_v2_config(obj);
            }
        }
    }

    // version === 0, -1, 1.5, "2"
    // (`None` is impossible here: a missing version migrates to v1 above.)
    let version = version.expect("missing version migrates above");
    Err(invalid_browser_config_error(format!(
        "Unsupported browser config version: {}",
        match version {
            Value::Null => "null".to_string(),
            v => v.to_string(),
        }
    )))
}

/// `migrateV1Config(raw)` — `{ endpoint?, activeUrl?, refs? }`.
fn migrate_v1_config(raw: &Map<String, Value>) -> Result<BrowserConfig, BrowserError> {
    let endpoint_raw = match raw.get("endpoint") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        _ => DEFAULT_ENDPOINT.to_string(),
    };

    if !is_http_url(&endpoint_raw) {
        return Err(invalid_browser_config_error(format!(
            "Invalid V1 endpoint: \"{endpoint_raw}\". Must be an http(s) URL."
        )));
    }

    Ok(BrowserConfig {
        version: CURRENT_CONFIG_VERSION,
        connection: BrowserConnectionConfig::Cdp {
            browser_kind: "chromium".to_string(), // Generic CDP — refined after /json/version
            endpoint: endpoint_raw,
        },
        active_url: optional_string(raw.get("activeUrl")),
        refs: validate_refs_map(raw.get("refs"))?,
        ..Default::default()
    })
}

/// `validateV2Config(raw)`.
fn validate_v2_config(raw: &Map<String, Value>) -> Result<BrowserConfig, BrowserError> {
    let conn = raw
        .get("connection")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_browser_config_error("connection field is required in v2 config"))?;

    let protocol = validate_enum(conn.get("protocol"), &["cdp", "webdriver"], "protocol")?;
    let endpoint = require_http_url(
        require_non_empty_string(conn.get("endpoint"), "connection.endpoint")?,
        "connection.endpoint",
    )?;

    let mut config = BrowserConfig {
        version: CURRENT_CONFIG_VERSION,
        active_url: optional_string(raw.get("activeUrl")),
        active_page_id: optional_string(raw.get("activePageId")),
        tab_order: validate_optional_string_array(raw.get("tabOrder"))?,
        refs: validate_refs_map(raw.get("refs"))?,
        refs_page_id: optional_string(raw.get("refsPageId")),
        refs_url: optional_string(raw.get("refsUrl")),
        ..Default::default()
    };

    if protocol == "cdp" {
        config.connection = BrowserConnectionConfig::Cdp {
            browser_kind: validate_enum(
                conn.get("browserKind"),
                &["chrome", "edge", "chromium"],
                "browser kind",
            )?,
            endpoint,
        };
        return Ok(config);
    }

    // Early Safari builds read only the root-level WebDriver sessionId, while
    // safaridriver returns it under value.sessionId. JSON serialization omitted
    // that undefined field, leaving a config no browser command could load.
    // Recover only that historical missing-field shape; malformed values should
    // still fail validation instead of being silently discarded.
    let browser_kind = validate_enum(conn.get("browserKind"), &["safari"], "browser kind")?;
    if conn.get("sessionId").is_none() {
        return Ok(default_browser_config());
    }

    config.connection = BrowserConnectionConfig::Webdriver {
        browser_kind,
        endpoint,
        session_id: require_non_empty_string(conn.get("sessionId"), "connection.sessionId")?,
        driver_pid: optional_positive_integer(conn.get("driverPid"))?,
    };
    Ok(config)
}

/// Serialize the config in the exact key order the TS constructs it.
pub fn config_to_json(config: &BrowserConfig) -> Value {
    let mut obj = Map::new();
    obj.insert("version".to_string(), json!(config.version));
    let mut conn = Map::new();
    conn.insert(
        "protocol".to_string(),
        Value::String(config.connection.protocol().to_string()),
    );
    conn.insert(
        "browserKind".to_string(),
        Value::String(config.connection.browser_kind().to_string()),
    );
    conn.insert(
        "endpoint".to_string(),
        Value::String(config.connection.endpoint().to_string()),
    );
    if let Some(sid) = config.connection.session_id() {
        conn.insert("sessionId".to_string(), Value::String(sid.to_string()));
    }
    if let BrowserConnectionConfig::Webdriver {
        driver_pid: Some(pid),
        ..
    } = config.connection
    {
        conn.insert("driverPid".to_string(), json!(pid));
    }
    obj.insert("connection".to_string(), Value::Object(conn));
    if let Some(v) = &config.active_url {
        obj.insert("activeUrl".to_string(), Value::String(v.clone()));
    }
    if let Some(v) = &config.active_page_id {
        obj.insert("activePageId".to_string(), Value::String(v.clone()));
    }
    if let Some(v) = &config.tab_order {
        obj.insert(
            "tabOrder".to_string(),
            Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
    if let Some(v) = &config.refs {
        obj.insert("refs".to_string(), Value::Object(v.clone()));
    }
    if let Some(v) = &config.refs_page_id {
        obj.insert("refsPageId".to_string(), Value::String(v.clone()));
    }
    if let Some(v) = &config.refs_url {
        obj.insert("refsUrl".to_string(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

// ── Validation helpers ──────────────────────────────────────────────

fn require_non_empty_string(value: Option<&Value>, field: &str) -> Result<String, BrowserError> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => Err(invalid_browser_config_error(format!(
            "{field} must be a non-empty string"
        ))),
    }
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn require_http_url(value: String, field: &str) -> Result<String, BrowserError> {
    if is_http_url(&value) {
        Ok(value)
    } else {
        Err(invalid_browser_config_error(format!(
            "{field} must be an http(s) URL, got: {}",
            serde_json::to_string(&value).unwrap_or_default()
        )))
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn optional_positive_integer(value: Option<&Value>) -> Result<Option<i64>, BrowserError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v > 0) => Ok(n.as_i64()),
        Some(v) => Err(invalid_browser_config_error(format!(
            "expected positive integer, got {v}"
        ))),
    }
}

fn validate_enum(
    value: Option<&Value>,
    allowed: &[&str],
    field: &str,
) -> Result<String, BrowserError> {
    match value {
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(s.clone()),
        other => Err(invalid_browser_config_error(format!(
            "Invalid {field}: \"{}\". Expected one of: {}",
            other
                .map(|v| v.to_string())
                .unwrap_or_else(|| "undefined".to_string()),
            allowed.join(", ")
        ))),
    }
}

fn validate_optional_string_array(
    value: Option<&Value>,
) -> Result<Option<Vec<String>>, BrowserError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
                    _ => {
                        return Err(invalid_browser_config_error(
                            "tabOrder must contain only non-empty strings",
                        ))
                    }
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(invalid_browser_config_error(
            "tabOrder must be an array of strings",
        )),
    }
}

fn validate_refs_map(value: Option<&Value>) -> Result<Option<Map<String, Value>>, BrowserError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if !v.is_string() {
                    return Err(invalid_browser_config_error(format!(
                        "refs[\"{k}\"] must be a string selector"
                    )));
                }
            }
            Ok(Some(map.clone()))
        }
        Some(_) => Err(invalid_browser_config_error(
            "refs must be a JSON object (string → string)",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(raw: Value) -> Result<BrowserConfig, BrowserError> {
        parse_browser_config(&raw)
    }

    #[test]
    fn empty_object_is_default_v2() {
        let config = parse(json!({})).unwrap();
        assert_eq!(config.version, 2);
        assert_eq!(config.connection.protocol(), "cdp");
        assert_eq!(config.connection.browser_kind(), "chromium");
        assert_eq!(config.connection.endpoint(), "http://127.0.0.1:9222");
    }

    #[test]
    fn version_1_migrates_to_v2() {
        let config = parse(json!({"version": 1, "endpoint": "http://127.0.0.1:9225"})).unwrap();
        assert_eq!(config.version, 2);
        assert_eq!(config.connection.protocol(), "cdp");
        assert_eq!(config.connection.browser_kind(), "chromium");
        assert_eq!(config.connection.endpoint(), "http://127.0.0.1:9225");
    }

    #[test]
    fn v1_without_endpoint_defaults() {
        let config = parse(json!({"version": 1})).unwrap();
        assert_eq!(config.connection.endpoint(), "http://127.0.0.1:9222");
    }

    #[test]
    fn v1_invalid_endpoint_throws() {
        assert!(parse(json!({"version": 1, "endpoint": "not-a-url"})).is_err());
    }

    #[test]
    fn valid_v2_cdp_passes() {
        let config = parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "http://127.0.0.1:9222"}
        }))
        .unwrap();
        assert_eq!(config.version, 2);
        assert_eq!(config.connection.protocol(), "cdp");
        assert_eq!(config.connection.browser_kind(), "chrome");
    }

    #[test]
    fn v2_with_active_page_and_tab_order_preserved() {
        let config = parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "edge", "endpoint": "http://127.0.0.1:9999"},
            "activePageId": "target-123",
            "tabOrder": ["target-123", "target-456"],
            "refs": {"b1": "#btn"},
            "refsPageId": "target-123"
        }))
        .unwrap();
        assert_eq!(config.active_page_id.as_deref(), Some("target-123"));
        assert_eq!(
            config.tab_order.as_deref(),
            Some(&["target-123".to_string(), "target-456".to_string()][..])
        );
        assert_eq!(
            config.refs.as_ref().unwrap().get("b1"),
            Some(&json!("#btn"))
        );
    }

    #[test]
    fn future_version_throws() {
        assert!(parse(json!({"version": 99})).is_err());
    }

    #[test]
    fn version_zero_throws() {
        assert!(parse(json!({"version": 0})).is_err());
    }

    #[test]
    fn version_as_string_throws() {
        assert!(parse(json!({"version": "2"})).is_err());
    }

    #[test]
    fn v2_without_connection_throws() {
        assert!(parse(json!({"version": 2})).is_err());
    }

    #[test]
    fn v2_invalid_protocol_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "banana", "browserKind": "chrome", "endpoint": "http://x"}
        }))
        .is_err());
    }

    #[test]
    fn v2_cdp_with_safari_browser_kind_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "safari", "endpoint": "http://x"}
        }))
        .is_err());
    }

    #[test]
    fn v2_webdriver_with_invalid_browser_kind_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "webdriver", "browserKind": "chrome", "endpoint": "http://x", "sessionId": "s1"}
        }))
        .is_err());
    }

    #[test]
    fn v2_webdriver_valid_config_passes() {
        let config = parse(json!({
            "version": 2,
            "connection": {
                "protocol": "webdriver",
                "browserKind": "safari",
                "endpoint": "http://127.0.0.1:4444",
                "sessionId": "abc-123",
                "driverPid": 45678
            }
        }))
        .unwrap();
        assert_eq!(config.connection.protocol(), "webdriver");
        let (session_id, driver_pid) = webdriver_fields(&config.connection);
        assert_eq!(session_id, "abc-123");
        assert_eq!(driver_pid, Some(45678));
    }

    /// Extract webdriver-only fields; `None` for CDP connections (both arms
    /// execute across the test suite, unlike an if-let panic branch).
    fn webdriver_fields(conn: &BrowserConnectionConfig) -> (String, Option<i64>) {
        match conn {
            BrowserConnectionConfig::Webdriver {
                session_id,
                driver_pid,
                ..
            } => (session_id.clone(), *driver_pid),
            BrowserConnectionConfig::Cdp { .. } => (String::new(), None),
        }
    }

    #[test]
    fn v2_safari_missing_session_id_recovers_to_defaults() {
        let config = parse(json!({
            "version": 2,
            "connection": {
                "protocol": "webdriver",
                "browserKind": "safari",
                "endpoint": "http://127.0.0.1:4444",
                "driverPid": 45678
            },
            "activePageId": "stale-safari-window"
        }))
        .unwrap();
        assert_eq!(config.version, 2);
        assert_eq!(config.connection.protocol(), "cdp");
        assert_eq!(config.connection.browser_kind(), "chromium");
        assert_eq!(config.connection.endpoint(), "http://127.0.0.1:9222");
    }

    #[test]
    fn v2_safari_blank_session_id_still_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {
                "protocol": "webdriver",
                "browserKind": "safari",
                "endpoint": "http://127.0.0.1:4444",
                "sessionId": "  "
            }
        }))
        .is_err());
    }

    #[test]
    fn v2_missing_endpoint_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome"}
        }))
        .is_err());
    }

    #[test]
    fn v2_endpoint_not_http_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "ftp://bad"}
        }))
        .is_err());
    }

    #[test]
    fn v2_tab_order_not_array_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "http://x"},
            "tabOrder": "not-an-array"
        }))
        .is_err());
    }

    #[test]
    fn v2_refs_with_non_string_values_throws() {
        assert!(parse(json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "http://x"},
            "refs": {"b1": 123}
        }))
        .is_err());
    }

    #[test]
    fn v1_refs_with_non_string_values_throws() {
        assert!(parse(json!({"version": 1, "refs": {"b1": 7}})).is_err());
    }

    #[cfg(not(windows))] // dirs::home_dir ignores env vars on Windows
    #[tokio::test(flavor = "multi_thread")]
    async fn browser_dir_falls_back_to_home_when_future_home_unset() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _home =
            crate::test_env::EnvGuard::set(&[("HOME", dir.path().as_os_str().to_os_string())]);
        let _no_fh = crate::test_env::EnvGuard::remove(&["FUTURE_HOME"]);
        assert_eq!(
            browser_dir(),
            dir.path().join(".future").join("agent").join("browser")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_fails_when_home_is_a_file_and_config_path_is_a_dir() {
        let _guard = crate::test_env::lock_env().await;

        // FUTURE_HOME pointing at a regular FILE → create_dir_all fails.
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("afile");
        std::fs::write(&file, "x").expect("write");
        let _env =
            crate::test_env::EnvGuard::set(&[("FUTURE_HOME", file.as_os_str().to_os_string())]);
        let err = save_browser_config(&BrowserConfig::default())
            .await
            .unwrap_err();
        assert!(!err.to_string().is_empty());
        drop(_env);

        // config.json existing as a DIRECTORY → the write fails.
        let tmp2 = tempfile::tempdir().expect("tempdir");
        let _env2 = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            tmp2.path().as_os_str().to_os_string(),
        )]);
        tokio::fs::create_dir_all(browser_dir().join("config.json"))
            .await
            .expect("mkdir");
        let err = save_browser_config(&BrowserConfig::default())
            .await
            .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn not_an_object_throws() {
        assert!(parse(json!(null)).is_err());
        assert!(parse(json!("invalid")).is_err());
        assert!(parse(json!([1, 2])).is_err());
    }

    #[test]
    fn default_config_shape() {
        let config = default_browser_config();
        let value = config_to_json(&config);
        assert_eq!(
            value,
            json!({
                "version": 2,
                "connection": {
                    "protocol": "cdp",
                    "browserKind": "chromium",
                    "endpoint": "http://127.0.0.1:9222",
                }
            })
        );
    }

    #[test]
    fn v2_config_roundtrip() {
        let raw = json!({
            "version": 2,
            "connection": {
                "protocol": "cdp",
                "browserKind": "chrome",
                "endpoint": "http://127.0.0.1:9333",
            },
            "activeUrl": "https://example.com",
            "refs": {"abc": "#button"},
        });
        let config = parse_browser_config(&raw).unwrap();
        assert_eq!(config.connection.browser_kind(), "chrome");
        assert_eq!(config.active_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.connection.endpoint(), "http://127.0.0.1:9333");
        let roundtrip = config_to_json(&config);
        assert_eq!(roundtrip.get("activeUrl"), raw.get("activeUrl"));
        assert_eq!(roundtrip.get("refs"), raw.get("refs"));
    }

    #[tokio::test]
    async fn load_save_roundtrip_via_future_home() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        assert_eq!(browser_dir(), dir.path().join("agent").join("browser"));
        // Missing file → default config.
        let loaded = load_browser_config().await.expect("default load");
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.connection.endpoint(), "http://127.0.0.1:9222");
        // Save then load a webdriver config.
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: "http://127.0.0.1:4444".to_string(),
                session_id: "s1".to_string(),
                driver_pid: Some(42),
            },
            active_url: Some("https://example.com".to_string()),
            active_page_id: Some("p1".to_string()),
            tab_order: Some(vec!["p1".to_string()]),
            refs: None,
            refs_page_id: Some("p1".to_string()),
            refs_url: Some("https://example.com".to_string()),
        };
        save_browser_config(&config).await.expect("save");
        let loaded = load_browser_config().await.expect("load saved");
        assert_eq!(loaded.connection.protocol(), "webdriver");
        assert_eq!(loaded.connection.session_id(), Some("s1"));
        assert_eq!(loaded.active_page_id.as_deref(), Some("p1"));
        assert_eq!(loaded.refs_url.as_deref(), Some("https://example.com"));
    }

    #[tokio::test]
    async fn load_rejects_invalid_json() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        let config_dir = browser_dir();
        tokio::fs::create_dir_all(&config_dir).await.expect("mkdir");
        tokio::fs::write(config_dir.join("config.json"), "not json")
            .await
            .expect("write");
        let err = load_browser_config().await.unwrap_err();
        assert_eq!(err.code, "invalid_config");
        assert!(err.message.contains("Invalid JSON"));
    }

    #[tokio::test]
    async fn load_maps_non_notfound_io_errors() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        // config.json as a DIRECTORY → read_to_string fails non-ENOENT.
        let config_dir = browser_dir();
        tokio::fs::create_dir_all(config_dir.join("config.json"))
            .await
            .expect("mkdir");
        let err = load_browser_config().await.unwrap_err();
        assert_eq!(err.code, "invalid_config");
        assert!(!err.message.contains("Invalid JSON"));
    }

    #[test]
    fn version_null_and_fractional_throw() {
        let err = parse(json!({"version": null})).unwrap_err();
        assert!(err.message.contains("null"));
        let err = parse(json!({"version": 1.5})).unwrap_err();
        assert!(err.message.contains("1.5"));
        let err = parse(json!({"version": -1})).unwrap_err();
        assert!(err.message.contains("-1"));
    }

    #[test]
    fn v1_active_url_and_refs_migrate() {
        let config = parse(json!({
            "version": 1,
            "endpoint": "http://127.0.0.1:9225",
            "activeUrl": "https://example.com",
            "refs": {"b1": "#btn"}
        }))
        .unwrap();
        assert_eq!(config.active_url.as_deref(), Some("https://example.com"));
        assert!(config.refs.is_some());
        // Blank endpoint falls back to the default.
        let config = parse(json!({"version": 1, "endpoint": "  "})).unwrap();
        assert_eq!(config.connection.endpoint(), "http://127.0.0.1:9222");
    }

    #[test]
    fn helper_edge_cases() {
        // optional_positive_integer: null → None, 0/negative/string → error.
        assert_eq!(optional_positive_integer(None).unwrap(), None);
        assert_eq!(optional_positive_integer(Some(&Value::Null)).unwrap(), None);
        assert!(optional_positive_integer(Some(&json!(0))).is_err());
        assert!(optional_positive_integer(Some(&json!(-3))).is_err());
        assert!(optional_positive_integer(Some(&json!("7"))).is_err());
        assert_eq!(optional_positive_integer(Some(&json!(7))).unwrap(), Some(7));
        // validate_enum: missing field renders "undefined".
        let err = validate_enum(None, &["a"], "thing").unwrap_err();
        assert!(err.message.contains("\"undefined\""));
        // tabOrder: empty-string item rejected.
        assert!(validate_optional_string_array(Some(&json!(["ok", " "]))).is_err());
        assert_eq!(
            validate_optional_string_array(Some(&Value::Null)).unwrap(),
            None
        );
        // refs: non-object rejected.
        assert!(validate_refs_map(Some(&json!(["x"]))).is_err());
        assert_eq!(validate_refs_map(Some(&Value::Null)).unwrap(), None);
        // require_http_url: non-string JSON rendering in message.
        let err = require_http_url("ftp://x".to_string(), "f").unwrap_err();
        assert!(err.message.contains("\"ftp://x\""));
    }

    #[test]
    fn webdriver_driver_pid_validation() {
        let base = |pid: Value| {
            json!({
                "version": 2,
                "connection": {
                    "protocol": "webdriver",
                    "browserKind": "safari",
                    "endpoint": "http://127.0.0.1:4444",
                    "sessionId": "s1",
                    "driverPid": pid
                }
            })
        };
        // Valid positive integer accepted; zero rejected.
        assert!(parse(base(json!(1))).is_ok());
        assert!(parse(base(json!(0))).is_err());
        // Absent → None.
        let config = parse(json!({
            "version": 2,
            "connection": {
                "protocol": "webdriver",
                "browserKind": "safari",
                "endpoint": "http://127.0.0.1:4444",
                "sessionId": "s1"
            }
        }))
        .unwrap();
        let (session_id, driver_pid) = webdriver_fields(&config.connection);
        assert_eq!(session_id, "s1");
        assert_eq!(driver_pid, None);
    }

    #[test]
    fn config_to_json_webdriver_includes_session_fields() {
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: "http://127.0.0.1:4444".to_string(),
                session_id: "s1".to_string(),
                driver_pid: Some(9),
            },
            ..Default::default()
        };
        let value = config_to_json(&config);
        let conn = value.get("connection").expect("connection");
        assert_eq!(conn.get("sessionId"), Some(&json!("s1")));
        assert_eq!(conn.get("driverPid"), Some(&json!(9)));
    }

    #[test]
    fn webdriver_fields_cdp_arm() {
        let cdp = BrowserConnectionConfig::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: "http://e".to_string(),
        };
        assert_eq!(webdriver_fields(&cdp), (String::new(), None));
        let wd = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: "http://e".to_string(),
            session_id: "s".to_string(),
            driver_pid: Some(3),
        };
        assert_eq!(webdriver_fields(&wd), ("s".to_string(), Some(3)));
    }
}
