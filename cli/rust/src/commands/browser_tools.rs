//! Browser tool — port of `cli/src/commands/browser-tools.ts` (P2 scope).
//!
//! P2 ports the CLI-visible surface and the two commands that need no CDP
//! WebSocket session: `start` (launch a Chromium browser with a remote
//! debugging port) and `status` (probe `/json/version`). The session-based
//! commands (`tabs` / `open` / `snapshot` / `click` / `type` / `press` /
//! `screenshot` / `scroll` / `console`) and the Safari path land with the
//! full browser subsystem (P3) and return a clear error meanwhile.
//!
//! Config state (`~/.future/agent/browser/config.json`) is ported from
//! `cli/src/browser/browser-state.ts` including the v1 → v2 migration and
//! runtime validation.

use crate::output::Output;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// Browser directory — `~/.future/agent/browser` (honors `FUTURE_HOME`).
fn browser_dir() -> PathBuf {
    let future_home = std::env::var("FUTURE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".future"));
    future_home.join("agent").join("browser")
}

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:9222";

/// `BrowserToolEntry` — description/args/example for the catalog.
pub struct BrowserToolEntry {
    pub description: &'static str,
    pub args: Vec<(&'static str, &'static str)>,
    pub example: &'static str,
}

/// `BROWSER_TOOL_CATALOG` — the single `browser` tool.
pub fn browser_tool_catalog() -> Vec<(&'static str, BrowserToolEntry)> {
    vec![(
        "browser",
        BrowserToolEntry {
            description: "Control a local Chrome/Edge/Safari browser for web automation: navigate pages, take snapshots, click elements, fill forms, capture screenshots.",
            args: vec![
                ("command", "sub-command: start | status | open | snapshot | click | type | press | scroll | screenshot | console | tabs (required)"),
                ("url", "URL to navigate to (for open / start)"),
                ("ref", "element reference from a previous snapshot (for click / type)"),
                ("text", "text to type into an element (for type)"),
                ("key", "key to press, e.g. \"Enter\" or \"Escape\" (for press)"),
                ("fullPage", "capture the full scrollable page (for screenshot, default: false)"),
                ("limit", "max snapshot lines to return (default: 80)"),
                ("path", "file path to save screenshot, e.g. ./page.png (for screenshot)"),
                ("level", "console message level to filter: \"log\" | \"warn\" | \"error\" (for console)"),
            ],
            example: "{\"command\": \"open\", \"url\": \"https://example.com\"}",
        },
    )]
}

/// `isBrowserTool(name)`.
pub fn is_browser_tool(name: &str) -> bool {
    name == "browser"
}

/// `LocalToolResult` — `{text?, structuredContent?}`.
pub struct LocalToolResult {
    pub text: Option<String>,
    pub structured_content: Option<Value>,
}

/// `callBrowserTool(name, args)` — dispatch on the `command` argument.
pub async fn call_browser_tool(
    _name: &str,
    args: &Map<String, Value>,
    _out: &Output,
) -> Result<LocalToolResult, String> {
    let command = string_arg(args, "command")
        .ok_or_else(|| "browser tool requires \"command\" argument.".to_string())?;

    match command.as_str() {
        "start" => browser_start(args).await,
        "status" => browser_status(args).await,
        "tabs" | "open" | "snapshot" | "click" | "type" | "press" | "screenshot" | "scroll"
        | "console" => Err(format!(
            "browser tool command '{command}' is not yet ported in the Rust CLI (P3 — browser session automation)."
        )),
        other => Err(format!(
            "Unknown browser command: \"{other}\". Use: start, status, tabs, open, snapshot, click, type, press, scroll, screenshot, console."
        )),
    }
}

// ── start ───────────────────────────────────────────────────────────────────

async fn browser_start(args: &Map<String, Value>) -> Result<LocalToolResult, String> {
    let requested_port = number_arg(args, "port").unwrap_or(9222.0) as i64;
    let browser_arg = string_arg(args, "browser");

    // Safari path — delegated to the browser subsystem (P3).
    if browser_arg.as_deref() == Some("safari") {
        return Err(
            "browser tool command 'start' for Safari is not yet ported in the Rust CLI (P3)."
                .to_string(),
        );
    }

    // Chrome/Edge/Chromium path
    let port = resolve_browser_port(requested_port).await?;
    let endpoint = format!("http://127.0.0.1:{port}");

    if endpoint_reachable(&endpoint).await {
        let mut config = load_browser_config().await?;
        let existing_endpoint = config.connection.endpoint.clone();
        config.connection.protocol = "cdp".to_string();
        config.connection.browser_kind = "chromium".to_string();
        config.connection.endpoint = endpoint.clone();
        save_browser_config(&config).await?;
        let note = if existing_endpoint.is_empty() || existing_endpoint == endpoint {
            "Browser is already running at this endpoint.".to_string()
        } else {
            format!(
                "Browser endpoint was updated (was {existing_endpoint}). Subsequent commands will use this browser."
            )
        };
        return Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "endpoint": endpoint,
                "status": "already_running",
                "note": note,
            })),
        });
    }

    let executable_path = string_arg(args, "executablePath");
    let launcher = find_browser_launcher(executable_path.as_deref());
    let Some(launcher) = launcher else {
        return Err(
            "Could not find Chrome or Edge. Pass executablePath to browser with command=start."
                .to_string(),
        );
    };

    let default_profile = browser_dir().join("profile");
    let profile_dir = match string_arg(args, "profileDir") {
        Some(dir) => PathBuf::from(dir),
        None if port == requested_port => default_profile,
        None => browser_dir().join(format!("profile-{port}")),
    };
    let url = string_arg(args, "url").unwrap_or_else(|| "about:blank".to_string());
    tokio::fs::create_dir_all(&profile_dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(browser_dir())
        .await
        .map_err(|e| e.to_string())?;

    let mut chrome_args = vec![
        format!("--remote-debugging-port={port}"),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        url.clone(),
    ];
    let mut browser_args = launcher.args.clone();
    browser_args.append(&mut chrome_args);

    #[cfg(windows)]
    {
        // PowerShell Windows-shell launcher so Chrome does not inherit the
        // agent's stdout handle (port of launchWindowsDetached).
        let script = format!(
            "Start-Process -FilePath '{}' -ArgumentList {} -WindowStyle Hidden",
            launcher.command,
            browser_args
                .iter()
                .map(|a| format!("'{}'", a.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let _ = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        // `spawn(..., { detached: true, stdio: "ignore" })` + `child.unref()`.
        let _ = tokio::process::Command::new(&launcher.command)
            .args(&browser_args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if endpoint_reachable(&endpoint).await {
            let mut cfg = load_browser_config().await?;
            cfg.connection.protocol = "cdp".to_string();
            cfg.connection.browser_kind = "chromium".to_string();
            cfg.connection.endpoint = endpoint.clone();
            cfg.active_url = Some(url);
            save_browser_config(&cfg).await?;
            return Ok(LocalToolResult {
                text: None,
                structured_content: Some(json!({
                    "endpoint": endpoint,
                    "launcher": { "command": launcher.command, "args": launcher.args },
                    "profileDir": profile_dir.display().to_string(),
                    "port": port,
                    "requestedPort": requested_port,
                    "status": "started",
                })),
            });
        }
        crate::utils::time::sleep(250).await;
    }

    let mut cfg2 = load_browser_config().await?;
    cfg2.connection.protocol = "cdp".to_string();
    cfg2.connection.browser_kind = "chromium".to_string();
    cfg2.connection.endpoint = endpoint.clone();
    cfg2.active_url = Some(url);
    save_browser_config(&cfg2).await?;
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "endpoint": endpoint,
            "launcher": { "command": launcher.command, "args": launcher.args },
            "profileDir": profile_dir.display().to_string(),
            "port": port,
            "requestedPort": requested_port,
            "status": "starting",
            "note": "Browser was launched, but the debugging endpoint did not answer within 10 seconds.",
        })),
    })
}

// ── status ──────────────────────────────────────────────────────────────────

async fn browser_status(args: &Map<String, Value>) -> Result<LocalToolResult, String> {
    let endpoint = endpoint_for(args).await;
    let client = reqwest::Client::new();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        client.get(format!("{endpoint}/json/version")).send(),
    )
    .await;
    match result {
        Ok(Ok(response)) if response.status().is_success() => {
            let version: Value = response.json().await.unwrap_or(Value::Null);
            Ok(LocalToolResult {
                text: None,
                structured_content: Some(json!({
                    "endpoint": endpoint,
                    "reachable": true,
                    "version": version,
                })),
            })
        }
        Ok(Ok(response)) => Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "endpoint": endpoint,
                "reachable": false,
                "status": response.status().as_u16(),
            })),
        }),
        _ => Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "endpoint": endpoint,
                "reachable": false,
                "error": "Local browser endpoint is not reachable.",
            })),
        }),
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

async fn endpoint_for(args: &Map<String, Value>) -> String {
    let config = load_browser_config().await.unwrap_or_default();
    string_arg(args, "endpoint").unwrap_or_else(|| {
        if config.connection.endpoint.is_empty() {
            DEFAULT_ENDPOINT.to_string()
        } else {
            config.connection.endpoint
        }
    })
}

async fn endpoint_reachable(endpoint: &str) -> bool {
    let client = reqwest::Client::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        client.get(format!("{endpoint}/json/version")).send(),
    )
    .await
    .map(|r| r.map(|resp| resp.status().is_success()).unwrap_or(false))
    .unwrap_or(false)
}

async fn port_has_listener(port: i64) -> bool {
    use tokio::io::AsyncWriteExt;
    let mut socket = match tokio::net::TcpStream::connect(("127.0.0.1", port as u16)).await {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = socket.shutdown().await;
    true
}

/// `resolveBrowserPort` — use the requested port if free/reachable, else scan
/// the next 49 ports for a free one.
async fn resolve_browser_port(requested_port: i64) -> Result<i64, String> {
    let endpoint = format!("http://127.0.0.1:{requested_port}");
    if endpoint_reachable(&endpoint).await {
        return Ok(requested_port);
    }
    if !port_has_listener(requested_port).await {
        return Ok(requested_port);
    }
    for port in requested_port + 1..requested_port + 50 {
        if !port_has_listener(port).await {
            return Ok(port);
        }
    }
    Err(format!(
        "No available browser debugging port found near {requested_port}."
    ))
}

/// Browser discovery — `findBrowser(executablePath?)` port.
struct Launcher {
    command: String,
    args: Vec<String>,
}

fn find_browser_launcher(executable_path: Option<&str>) -> Option<Launcher> {
    let exe = executable_path.map(|p| {
        let kind = infer_kind(p);
        (kind, p.to_string())
    });
    let found = exe.or_else(find_discovered);
    found.map(|(_kind, path)| Launcher {
        command: path,
        args: Vec::new(),
    })
}

fn infer_kind(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.contains("edge") {
        "edge"
    } else if lower.contains("chromium") {
        "chromium"
    } else {
        "chrome"
    }
}

fn find_discovered() -> Option<(&'static str, String)> {
    #[cfg(target_os = "macos")]
    {
        const CANDIDATES: [(&str, &str); 3] = [
            (
                "chrome",
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ),
            (
                "edge",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ),
            (
                "chromium",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            ),
        ];
        for (kind, path) in CANDIDATES {
            if std::path::Path::new(path).exists() {
                return Some((kind, path.to_string()));
            }
        }
        None
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA").ok();
        let prog = std::env::var("PROGRAMFILES").ok();
        let prog_x86 = std::env::var("PROGRAMFILES(X86)").ok();
        let candidates = [
            (
                "chrome",
                local.map(|p| format!("{p}\\Google\\Chrome\\Application\\chrome.exe")),
            ),
            (
                "chrome",
                prog.map(|p| format!("{p}\\Google\\Chrome\\Application\\chrome.exe")),
            ),
            (
                "edge",
                prog_x86.map(|p| format!("{p}\\Microsoft\\Edge\\Application\\msedge.exe")),
            ),
            (
                "edge",
                prog.map(|p| format!("{p}\\Microsoft\\Edge\\Application\\msedge.exe")),
            ),
        ];
        for (kind, path) in candidates {
            if let Some(path) = path {
                if std::path::Path::new(&path).exists() {
                    return Some((kind, path));
                }
            }
        }
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        const CANDIDATES: [(&str, &str); 4] = [
            ("chrome", "/usr/bin/google-chrome"),
            ("chrome", "/usr/bin/chromium-browser"),
            ("chromium", "/usr/bin/chromium"),
            ("edge", "/usr/bin/microsoft-edge"),
        ];
        for (kind, path) in CANDIDATES {
            if std::path::Path::new(path).exists() {
                return Some((kind, path.to_string()));
            }
        }
        return None;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

// ── config state (port of browser-state.ts) ─────────────────────────────────

const CURRENT_CONFIG_VERSION: i64 = 2;

#[derive(Debug, Clone)]
pub struct BrowserConnection {
    pub protocol: String,
    pub browser_kind: String,
    pub endpoint: String,
    pub session_id: Option<String>,
    pub driver_pid: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserConfig {
    pub version: i64,
    pub connection: BrowserConnection,
    pub active_url: Option<String>,
    pub active_page_id: Option<String>,
    pub tab_order: Option<Vec<String>>,
    pub refs: Option<Map<String, Value>>,
    pub refs_page_id: Option<String>,
    pub refs_url: Option<String>,
}

impl Default for BrowserConnection {
    fn default() -> Self {
        Self {
            protocol: "cdp".to_string(),
            browser_kind: "chromium".to_string(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
            session_id: None,
            driver_pid: None,
        }
    }
}

/// `defaultBrowserConfig()`.
fn default_browser_config() -> BrowserConfig {
    BrowserConfig {
        version: CURRENT_CONFIG_VERSION,
        connection: BrowserConnection::default(),
        ..Default::default()
    }
}

/// `loadBrowserConfig()`.
pub async fn load_browser_config() -> Result<BrowserConfig, String> {
    let config_file = browser_dir().join("config.json");
    match tokio::fs::read_to_string(&config_file).await {
        Ok(raw) => {
            let parsed: Value = serde_json::from_str(&raw)
                .map_err(|_| "Invalid browser config: not JSON".to_string())?;
            parse_browser_config(&parsed)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default_browser_config()),
        Err(e) => Err(format!("Invalid browser config: {e}")),
    }
}

/// `saveBrowserConfig(config)`.
pub async fn save_browser_config(config: &BrowserConfig) -> Result<(), String> {
    tokio::fs::create_dir_all(browser_dir())
        .await
        .map_err(|e| e.to_string())?;
    let value = config_to_json(config);
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    tokio::fs::write(browser_dir().join("config.json"), text)
        .await
        .map_err(|e| e.to_string())
}

/// Serialize the config in the exact key order the TS constructs it.
fn config_to_json(config: &BrowserConfig) -> Value {
    let mut obj = Map::new();
    obj.insert("version".to_string(), json!(config.version));
    let mut conn = Map::new();
    conn.insert(
        "protocol".to_string(),
        Value::String(config.connection.protocol.clone()),
    );
    conn.insert(
        "browserKind".to_string(),
        Value::String(config.connection.browser_kind.clone()),
    );
    conn.insert(
        "endpoint".to_string(),
        Value::String(config.connection.endpoint.clone()),
    );
    if let Some(sid) = &config.connection.session_id {
        conn.insert("sessionId".to_string(), Value::String(sid.clone()));
    }
    if let Some(pid) = config.connection.driver_pid {
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

/// `parseBrowserConfig(raw)` with v1 migration and v2 validation.
fn parse_browser_config(raw: &Value) -> Result<BrowserConfig, String> {
    let Some(obj) = raw.as_object() else {
        return Err("Invalid browser config: Browser config must be a JSON object".to_string());
    };

    let version = obj.get("version");
    match version {
        None => migrate_v1_config(obj),
        Some(Value::Number(n)) if n.as_i64() == Some(1) => migrate_v1_config(obj),
        Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v > CURRENT_CONFIG_VERSION) => {
            Err(format!(
                "Invalid browser config: Unsupported browser config version: {}. Expected ≤ {}.",
                n, CURRENT_CONFIG_VERSION
            ))
        }
        Some(Value::Number(n)) if n.as_i64() == Some(CURRENT_CONFIG_VERSION) => {
            validate_v2_config(obj)
        }
        other => Err(format!(
            "Invalid browser config: Unsupported browser config version: {}",
            match other {
                Some(Value::Null) => "null".to_string(),
                Some(v) => v.to_string(),
                None => "undefined".to_string(),
            }
        )),
    }
}

fn migrate_v1_config(raw: &Map<String, Value>) -> Result<BrowserConfig, String> {
    let endpoint_raw = match raw.get("endpoint") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        _ => DEFAULT_ENDPOINT.to_string(),
    };
    if !endpoint_raw.starts_with("http://") && !endpoint_raw.starts_with("https://") {
        return Err(format!(
            "Invalid browser config: Invalid V1 endpoint: \"{endpoint_raw}\". Must be an http(s) URL."
        ));
    }
    Ok(BrowserConfig {
        version: CURRENT_CONFIG_VERSION,
        connection: BrowserConnection {
            protocol: "cdp".to_string(),
            browser_kind: "chromium".to_string(),
            endpoint: endpoint_raw,
            ..Default::default()
        },
        active_url: optional_string(raw.get("activeUrl")),
        refs: validate_refs_map(raw.get("refs"))?,
        ..Default::default()
    })
}

fn validate_v2_config(raw: &Map<String, Value>) -> Result<BrowserConfig, String> {
    let conn = raw
        .get("connection")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "Invalid browser config: connection field is required in v2 config".to_string()
        })?;

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
        config.connection = BrowserConnection {
            protocol: "cdp".to_string(),
            browser_kind: validate_enum(
                conn.get("browserKind"),
                &["chrome", "edge", "chromium"],
                "browser kind",
            )?,
            endpoint,
            ..Default::default()
        };
        return Ok(config);
    }

    // Early Safari builds read only the root-level WebDriver sessionId; recover
    // only that historical missing-field shape (port of the TS behavior).
    let browser_kind = validate_enum(conn.get("browserKind"), &["safari"], "browser kind")?;
    if conn.get("sessionId").is_none() {
        return Ok(default_browser_config());
    }
    config.connection = BrowserConnection {
        protocol: "webdriver".to_string(),
        browser_kind,
        endpoint,
        session_id: Some(require_non_empty_string(
            conn.get("sessionId"),
            "connection.sessionId",
        )?),
        driver_pid: optional_positive_integer(conn.get("driverPid"))?,
    };
    Ok(config)
}

fn require_non_empty_string(value: Option<&Value>, field: &str) -> Result<String, String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => Err(format!(
            "Invalid browser config: {field} must be a non-empty string"
        )),
    }
}

fn require_http_url(value: String, field: &str) -> Result<String, String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(value)
    } else {
        Err(format!(
            "Invalid browser config: {field} must be an http(s) URL, got: {}",
            serde_json::to_string(&value).unwrap_or_default()
        ))
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn optional_positive_integer(value: Option<&Value>) -> Result<Option<i64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v > 0) => Ok(n.as_i64()),
        Some(_) => Err(format!(
            "Invalid browser config: expected positive integer, got {}",
            value.unwrap()
        )),
    }
}

fn validate_enum(value: Option<&Value>, allowed: &[&str], field: &str) -> Result<String, String> {
    match value {
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(s.clone()),
        other => Err(format!(
            "Invalid browser config: Invalid {field}: \"{}\". Expected one of: {}",
            other
                .map(|v| v.to_string())
                .unwrap_or_else(|| "undefined".to_string()),
            allowed.join(", ")
        )),
    }
}

fn validate_optional_string_array(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
                    _ => {
                        return Err(
                            "Invalid browser config: tabOrder must contain only non-empty strings"
                                .to_string(),
                        )
                    }
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err("Invalid browser config: tabOrder must be an array of strings".to_string()),
    }
}

fn validate_refs_map(value: Option<&Value>) -> Result<Option<Map<String, Value>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => {
            for v in map.values() {
                if !v.is_string() {
                    return Err(format!(
                        "Invalid browser config: refs[\"{}\"] must be a string selector",
                        map.iter()
                            .find(|(_, val)| !val.is_string())
                            .map(|(k, _)| k.as_str())
                            .unwrap_or("")
                    ));
                }
            }
            Ok(Some(map.clone()))
        }
        Some(_) => {
            Err("Invalid browser config: refs must be a JSON object (string → string)".to_string())
        }
    }
}

// ── arg helpers ─────────────────────────────────────────────────────────────

fn string_arg(args: &Map<String, Value>, key: &str) -> Option<String> {
    match args.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn number_arg(args: &Map<String, Value>, key: &str) -> Option<f64> {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_f64().filter(|v| v.is_finite()),
        _ => None,
    }
}

/// Used by the session-based commands (tabs/type/press/screenshot) in P3.
#[allow(dead_code)]
fn boolean_arg(args: &Map<String, Value>, key: &str) -> Option<bool> {
    match args.get(key) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(config.connection.browser_kind, "chrome");
        assert_eq!(config.active_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.connection.endpoint, "http://127.0.0.1:9333");
        let roundtrip = config_to_json(&config);
        assert_eq!(roundtrip.get("activeUrl"), raw.get("activeUrl"));
        assert_eq!(roundtrip.get("refs"), raw.get("refs"));
    }

    #[test]
    fn v1_config_migrates() {
        let raw = json!({
            "endpoint": "http://127.0.0.1:9444",
            "activeUrl": "https://x.com",
        });
        let config = parse_browser_config(&raw).unwrap();
        assert_eq!(config.version, 2);
        assert_eq!(config.connection.protocol, "cdp");
        assert_eq!(config.connection.endpoint, "http://127.0.0.1:9444");
        assert_eq!(config.active_url.as_deref(), Some("https://x.com"));
    }

    #[test]
    fn invalid_configs_rejected() {
        // Not an object
        assert!(parse_browser_config(&json!([1, 2])).is_err());
        // Future version
        assert!(parse_browser_config(&json!({"version": 99})).is_err());
        // Bad v2 endpoint
        assert!(parse_browser_config(&json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "ftp://x"}
        }))
        .is_err());
        // Bad browser kind
        assert!(parse_browser_config(&json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "netscape", "endpoint": "http://x"}
        }))
        .is_err());
        // Bad refs value
        assert!(parse_browser_config(&json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "http://x"},
            "refs": {"a": 42}
        }))
        .is_err());
    }

    #[test]
    fn browser_catalog_shape() {
        let catalog = browser_tool_catalog();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].0, "browser");
        assert!(is_browser_tool("browser"));
        assert!(!is_browser_tool("search_paper"));
    }
}
