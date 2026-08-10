//! Browser tool — 1:1 port of `cli/src/commands/browser-tools.ts`.
//!
//! Full surface: `start` / `status` plus the session-based commands
//! (`tabs` / `open` / `snapshot` / `click` / `type` / `press` /
//! `screenshot` / `scroll` / `console`) over the browser subsystem in
//! `crate::browser` (P3).

use crate::browser::backend::{
    BrowserSession, BrowserSessionParams, CaptureScreenshotOptions, ClickOptions, EvaluateRequest,
    OpenPageOptions, PressOptions, ResolvedTarget, TabsAction, TypeOptions,
};
use crate::browser::browser_state::{load_browser_config, save_browser_config};
use crate::browser::chromium::chromium_endpoint::resolve_cdp_endpoint;
use crate::browser::chromium::chromium_manager::{
    endpoint_reachable, find_browser_launcher, resolve_port,
};
use crate::browser::safari::safari_manager::safari_start;
use crate::browser::screenshot_writer::{browser_dir, resolve_screenshot_path, write_screenshot};
use crate::browser::scripts::SNAPSHOT_FUNCTION_SOURCE;
use crate::browser::selector::resolve_target;
use crate::browser::types::{BrowserConfig, BrowserConnectionConfig, DEFAULT_TIMEOUTS};
use crate::output::Output;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:9222";
const DEFAULT_PROFILE_DIR: &str = "profile";

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

/// Test-only override for the browser-launcher lookup (dev machines with a
/// real Chrome install can never reach the no-launcher error arm).
#[cfg(test)]
static BROWSER_LAUNCHER_OVERRIDE: std::sync::Mutex<Option<Option<(String, String)>>> =
    std::sync::Mutex::new(None);

/// `findBrowserLauncher`, honoring the test override.
fn launcher_for(executable_path: Option<&str>) -> Option<(String, String)> {
    #[cfg(test)]
    if let Some(value) = BROWSER_LAUNCHER_OVERRIDE.lock().unwrap().clone() {
        return value;
    }
    find_browser_launcher(executable_path)
}

/// `LocalToolResult` — `{text?, structuredContent?}`.
#[derive(Debug)]
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
        "tabs" => with_session(args, |ctx| Box::pin(browser_tabs(ctx))).await,
        "open" => with_session(args, |ctx| Box::pin(browser_open(ctx))).await,
        "snapshot" => with_session(args, |ctx| Box::pin(browser_snapshot(ctx))).await,
        "click" => with_session(args, |ctx| Box::pin(browser_click(ctx))).await,
        "type" => with_session(args, |ctx| Box::pin(browser_type(ctx))).await,
        "press" => with_session(args, |ctx| Box::pin(browser_press(ctx))).await,
        "screenshot" => with_session(args, |ctx| Box::pin(browser_screenshot(ctx))).await,
        "scroll" => with_session(args, |ctx| Box::pin(browser_scroll(ctx))).await,
        "console" => with_session(args, |ctx| Box::pin(browser_console(ctx))).await,
        other => Err(format!(
            "Unknown browser command: \"{other}\". Use: start, status, tabs, open, snapshot, click, type, press, scroll, screenshot, console."
        )),
    }
}

// ── start ───────────────────────────────────────────────────────────

async fn browser_start(args: &Map<String, Value>) -> Result<LocalToolResult, String> {
    let requested_port = number_arg(args, "port").unwrap_or(9222.0) as i64;
    let browser_arg = string_arg(args, "browser");

    // Safari path — delegate to SafariManager.
    if browser_arg.as_deref() == Some("safari") {
        return browser_start_safari(args, requested_port).await;
    }

    // Chrome/Edge/Chromium path
    let port = resolve_port(requested_port).await?;
    let endpoint = format!("http://127.0.0.1:{port}");

    if endpoint_reachable(&endpoint).await {
        let mut config = load_browser_config().await?;
        let existing_endpoint = config.connection.endpoint().to_string();
        config.connection = BrowserConnectionConfig::Cdp {
            browser_kind: "chromium".to_string(),
            endpoint: endpoint.clone(),
        };
        save_browser_config(&config).await?;
        let note = if !existing_endpoint.is_empty() && existing_endpoint != endpoint {
            format!(
                "Browser endpoint was updated (was {existing_endpoint}). Subsequent commands will use this browser."
            )
        } else {
            "Browser is already running at this endpoint.".to_string()
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
    let launcher = launcher_for(executable_path.as_deref());
    let Some((command, _kind)) = launcher else {
        return Err(
            "Could not find Chrome or Edge. Pass executablePath to browser with command=start."
                .to_string(),
        );
    };

    let profile_dir = match string_arg(args, "profileDir") {
        Some(dir) => PathBuf::from(dir),
        None if port == requested_port => browser_dir().join(DEFAULT_PROFILE_DIR),
        None => browser_dir().join(format!("profile-{port}")),
    };
    let url = string_arg(args, "url").unwrap_or_else(|| "about:blank".to_string());
    tokio::fs::create_dir_all(&profile_dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(browser_dir())
        .await
        .map_err(|e| e.to_string())?;

    let chrome_args = vec![
        format!("--remote-debugging-port={port}"),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        url.clone(),
    ];

    #[cfg(windows)]
    {
        // PowerShell Windows-shell launcher so Chrome does not inherit the
        // agent's stdout handle (launchWindowsDetached).
        crate::browser::windows_process::launch_windows_detached(&command, &chrome_args).await?;
    }
    #[cfg(not(windows))]
    {
        // `spawn(..., { detached: true, stdio: "ignore" })` + `child.unref()`.
        let _ = tokio::process::Command::new(&command)
            .args(&chrome_args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if endpoint_reachable(&endpoint).await {
            let mut cfg = load_browser_config().await?;
            cfg.connection = BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: endpoint.clone(),
            };
            cfg.active_url = Some(url);
            save_browser_config(&cfg).await?;
            return Ok(LocalToolResult {
                text: None,
                structured_content: Some(json!({
                    "endpoint": endpoint,
                    "launcher": { "command": command, "args": Vec::<String>::new() },
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
    cfg2.connection = BrowserConnectionConfig::Cdp {
        browser_kind: "chromium".to_string(),
        endpoint: endpoint.clone(),
    };
    cfg2.active_url = Some(url);
    save_browser_config(&cfg2).await?;
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "endpoint": endpoint,
            "launcher": { "command": command, "args": Vec::<String>::new() },
            "profileDir": profile_dir.display().to_string(),
            "port": port,
            "requestedPort": requested_port,
            "status": "starting",
            "note": "Browser was launched, but the debugging endpoint did not answer within 10 seconds.",
        })),
    })
}

/// Safari start path (browser-tools.ts `if (browserArg === "safari")`).
async fn browser_start_safari(
    args: &Map<String, Value>,
    requested_port: i64,
) -> Result<LocalToolResult, String> {
    let result = safari_start(requested_port, string_arg(args, "url").as_deref()).await;
    match result {
        Ok(result) => {
            // Persist connection config.
            if result.connection.protocol() == "webdriver" {
                let mut config = load_browser_config().await?;
                config.connection = result.connection.clone();
                config.active_url = string_arg(args, "url");
                save_browser_config(&config).await?;
            }
            Ok(LocalToolResult {
                text: None,
                structured_content: Some(json!({
                    "endpoint": result.connection.endpoint(),
                    "launcher": result.launcher,
                    "port": result.port,
                    "status": result.status,
                    "browserKind": "safari",
                })),
            })
        }
        Err(e) => {
            if is_permission_error(&e) {
                Ok(LocalToolResult {
                    text: None,
                    structured_content: Some(json!({
                        "status": "permission_required",
                        "browserKind": "safari",
                        "actionRequired": {
                            "description": "Safari remote automation is not enabled. This is a one-time setup.",
                            "steps": [
                                "Open Terminal and run: safaridriver --enable",
                                "You may be prompted for your password or to confirm in System Settings.",
                            ],
                            "command": "safaridriver --enable",
                        },
                    })),
                })
            } else {
                Err(e)
            }
        }
    }
}

fn is_permission_error(e: &str) -> bool {
    // SafariManager.translateError → BrowserPermissionError. The message is
    // "Safari remote automation is disabled." with code browser_permission_error.
    // Detect via the marker we emit in safari_manager::permission_error().
    e == "Safari remote automation is disabled."
}

// ── status ──────────────────────────────────────────────────────────

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

// ── BrowserSession context ──────────────────────────────────────────

struct SessionContext {
    session: Box<dyn BrowserSession>,
    config: BrowserConfig,
    /// Cloned copy of the tool args for the command body (avoids borrowing
    /// the caller's map across the HRTB closure boundary).
    args: Map<String, Value>,
}

/// `createSession(config, endpoint)`.
async fn create_session(
    config: &BrowserConfig,
    endpoint: &str,
) -> Result<Box<dyn BrowserSession>, String> {
    match &config.connection {
        BrowserConnectionConfig::Cdp { browser_kind, .. } => {
            // Refine browserKind from /json/version.
            let mut browser_kind = browser_kind.clone();
            if browser_kind == "chromium" {
                let refined = resolve_cdp_endpoint(endpoint, 5_000).await;
                if let Ok(info) = refined {
                    browser_kind = info.browser_kind;
                    // Atomically update config.
                    if let Ok(fresh) = load_browser_config().await {
                        if fresh.connection.protocol() == "cdp"
                            && fresh.connection.browser_kind() == "chromium"
                        {
                            let mut updated = fresh;
                            updated.connection = BrowserConnectionConfig::Cdp {
                                browser_kind: browser_kind.clone(),
                                endpoint: updated.connection.endpoint().to_string(),
                            };
                            let _ = save_browser_config(&updated).await;
                        }
                    }
                }
                // On error: keep "chromium".
            }
            crate::browser::create_default_session(BrowserSessionParams::Cdp {
                browser_kind,
                endpoint: endpoint.to_string(),
                timeouts: DEFAULT_TIMEOUTS,
                active_page_id: config.active_page_id.clone(),
                init_tab_order: config.tab_order.clone(),
            })
        }
        BrowserConnectionConfig::Webdriver { session_id, .. } => {
            if session_id.is_empty() {
                return Err("sessionId required for webdriver".to_string());
            }
            crate::browser::create_default_session(BrowserSessionParams::Webdriver {
                endpoint: endpoint.to_string(),
                session_id: session_id.clone(),
                timeouts: DEFAULT_TIMEOUTS,
                active_page_id: config.active_page_id.clone(),
            })
        }
    }
}

type SessionCommand<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<LocalToolResult, String>> + Send + 'a>,
>;

/// `withSession(args, fn)` — `fn(ctx, args)` runs with a live session.
async fn with_session<F>(args: &Map<String, Value>, f: F) -> Result<LocalToolResult, String>
where
    F: for<'a> FnOnce(&'a mut SessionContext) -> SessionCommand<'a>,
{
    let config = load_browser_config().await?;

    let mut endpoint = ensure_browser(args).await?;
    let session: Box<dyn BrowserSession> = match create_session(&config, &endpoint).await {
        Ok(s) => s,
        Err(error) => {
            if string_arg(args, "endpoint").is_some() {
                return Err(error);
            }
            // Auto-start and retry.
            let fallback_port = (port_from_endpoint(&endpoint).unwrap_or(9222)) + 1;
            let mut retry_args = args.clone();
            retry_args.insert("port".to_string(), json!(fallback_port));
            browser_start(&retry_args).await?;
            endpoint =
                wait_for_saved_endpoint(&format!("http://127.0.0.1:{fallback_port}"), 10_000)
                    .await?;
            create_session(&config, &endpoint).await?
        }
    };

    let mut ctx = SessionContext {
        session,
        config,
        args: args.clone(),
    };
    let result = f(&mut ctx).await;
    ctx.session.disconnect().await.ok();
    result
}

// ── Browser Tabs ────────────────────────────────────────────────────

async fn browser_tabs(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let action = string_arg(&ctx.args, "action").unwrap_or_else(|| "list".to_string());

    if action == "list" {
        let result = ctx.session.tabs(&TabsAction::List).await?;
        let tabs = match result {
            crate::browser::backend::InternalTabsResult::List { tabs } => tabs
                .iter()
                .map(|tab| {
                    json!({
                        "index": tab.index,
                        "title": tab.title,
                        "url": tab.url,
                        "active": tab.active,
                    })
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        return Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "tabs": tabs,
                "tabCount": tabs.len(),
            })),
        });
    }

    if action == "new" {
        let url = string_arg(&ctx.args, "url");
        let result = ctx.session.tabs(&TabsAction::New { url }).await?;
        let (page, index) = match result {
            crate::browser::backend::InternalTabsResult::New { page, index } => (page, index),
            _ => return Err("Unexpected tabs result".to_string()),
        };
        save_active_page(&page.url, Some(&page.page_id)).await?;
        // Refresh full tab list so the response shape matches "list".
        let tabs = list_tabs(ctx).await?;
        return Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "tabs": tabs.0,
                "tabCount": tabs.1,
                "created": { "index": index, "url": page.url },
            })),
        });
    }

    let index = number_arg(&ctx.args, "index");
    let Some(index) = index else {
        return Err(format!(
            "browser command tabs: action \"{action}\" requires a valid 0-based index."
        ));
    };
    if index < 0.0 {
        return Err(format!(
            "browser command tabs: action \"{action}\" requires a valid 0-based index."
        ));
    }
    let index = index as usize;

    if action == "select" {
        let result = ctx.session.tabs(&TabsAction::Select { index }).await?;
        let page = match result {
            crate::browser::backend::InternalTabsResult::Select { page } => page,
            _ => return Err("Unexpected tabs result".to_string()),
        };
        save_active_page(&page.url, Some(&page.page_id)).await?;
        let tabs = list_tabs(ctx).await?;
        return Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "tabs": tabs.0,
                "tabCount": tabs.1,
                "selected": { "index": index, "url": page.url },
            })),
        });
    }

    if action == "close" {
        let result = ctx.session.tabs(&TabsAction::Close { index }).await?;
        let url = match result {
            crate::browser::backend::InternalTabsResult::Close { url, .. } => url,
            _ => return Err("Unexpected tabs result".to_string()),
        };
        let tabs = list_tabs(ctx).await?;
        return Ok(LocalToolResult {
            text: None,
            structured_content: Some(json!({
                "tabs": tabs.0,
                "tabCount": tabs.1,
                "closed": { "index": index, "url": url },
            })),
        });
    }

    Err(
        "browser command tabs: action must be \"list\", \"new\", \"select\", or \"close\"."
            .to_string(),
    )
}

/// `(tabs, tabCount)` — the full tab list in the "list" shape.
async fn list_tabs(ctx: &mut SessionContext) -> Result<(Vec<Value>, usize), String> {
    let result = ctx.session.tabs(&TabsAction::List).await?;
    let tabs = match result {
        crate::browser::backend::InternalTabsResult::List { tabs } => tabs
            .iter()
            .map(|tab| {
                json!({
                    "index": tab.index,
                    "title": tab.title,
                    "url": tab.url,
                    "active": tab.active,
                })
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let count = tabs.len();
    Ok((tabs, count))
}

// ── Browser Open ────────────────────────────────────────────────────

async fn browser_open(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let url = string_arg(&ctx.args, "url")
        .ok_or_else(|| "browser command open requires url.".to_string())?;
    let page = ctx.session.open(&url, OpenPageOptions::default()).await?;
    clear_refs().await?;
    save_active_page(&page.url, Some(&page.page_id)).await?;
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "title": page.title,
            "url": page.url,
        })),
    })
}

// ── Browser Snapshot ────────────────────────────────────────────────

async fn browser_snapshot(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let limit = number_arg(&ctx.args, "limit").unwrap_or(80.0);
    let snapshot: Value = ctx
        .session
        .evaluate(&EvaluateRequest::Function {
            function_declaration: SNAPSHOT_FUNCTION_SOURCE.to_string(),
            // JS serializes integral numbers without ".0" — match JSON.stringify.
            arguments: vec![js_number(limit)],
        })
        .await?;

    let title = snapshot
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let url = snapshot
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let items = snapshot
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut refs: Map<String, Value> = Map::new();
    let mut lines: Vec<String> = Vec::new();
    let mut elements: Vec<Value> = Vec::new();
    for item in &items {
        let r#ref = item
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let selector = item
            .get("selector")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let disabled = item
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let checked = item.get("checked").cloned();
        let href = item.get("href").and_then(Value::as_str).map(str::to_string);

        refs.insert(r#ref.clone(), Value::String(selector.clone()));

        // state = [disabled?, checked?, href?].filter(Boolean).join(" ")
        let mut state_parts: Vec<String> = Vec::new();
        if disabled {
            state_parts.push("disabled".to_string());
        }
        if let Some(c) = &checked {
            if !c.is_null() {
                state_parts.push(format!("checked={c}"));
            }
        }
        if let Some(h) = &href {
            if !h.is_empty() {
                state_parts.push(format!("href={h}"));
            }
        }
        let state = state_parts.join(" ");
        let line = if state.is_empty() {
            format!("- {role} \"{name}\" [ref={}]", r#ref)
        } else {
            format!("- {role} \"{name}\" [ref={}] {state}", r#ref)
        };
        lines.push(line);

        // elements: item without selector, in TS key order.
        let mut element = Map::new();
        element.insert("ref".to_string(), json!(r#ref));
        element.insert("role".to_string(), json!(role));
        element.insert("name".to_string(), json!(name));
        element.insert(
            "tag".to_string(),
            item.get("tag").cloned().unwrap_or(Value::Null),
        );
        element.insert("disabled".to_string(), json!(disabled));
        element.insert("checked".to_string(), checked.unwrap_or(Value::Null));
        element.insert(
            "href".to_string(),
            href.map(Value::String).unwrap_or(Value::Null),
        );
        elements.push(Value::Object(element));
    }

    save_refs_and_url(&refs, &url).await?;

    let mut text_lines = Vec::new();
    text_lines.push(format!("Page: {title}"));
    text_lines.push(format!("URL: {url}"));
    text_lines.push(String::new());
    text_lines.extend(lines);

    Ok(LocalToolResult {
        text: Some(text_lines.join("\n")),
        structured_content: Some(json!({
            "title": title,
            "url": url,
            "elements": elements,
        })),
    })
}

// ── Browser Click ──────────────────────────────────────────────────

async fn browser_click(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let target = resolve_target_from_args(&ctx.args, &ctx.config)?;
    let result = ctx.session.click(&target, ClickOptions::default()).await?;
    save_active_page(&result.url, Some(&result.page_id)).await?;
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "clicked": target.original,
            "selector": target.selector,
            "title": result.title,
            "url": result.url,
        })),
    })
}

// ── Browser Type ───────────────────────────────────────────────────

async fn browser_type(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let text = string_arg(&ctx.args, "text")
        .ok_or_else(|| "browser command type requires text.".to_string())?;
    let target = resolve_target_from_args(&ctx.args, &ctx.config)?;
    let clear = boolean_arg(&ctx.args, "clear").unwrap_or(true);
    let submit = boolean_arg(&ctx.args, "submit").unwrap_or(false);
    let result = ctx
        .session
        .r#type(
            &target,
            &text,
            TypeOptions {
                clear: Some(clear),
                submit: Some(submit),
                timeout_ms: None,
            },
        )
        .await?;
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "typed": target.original,
            "selector": target.selector,
            "submitted": result.submitted,
        })),
    })
}

// ── Browser Press ──────────────────────────────────────────────────

async fn browser_press(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let key = string_arg(&ctx.args, "key")
        .ok_or_else(|| "browser command press requires key.".to_string())?;
    let target = resolve_target_from_args_optional(&ctx.args, &ctx.config)?;
    let result = ctx
        .session
        .press(&key, target.as_ref(), PressOptions::default())
        .await?;
    save_active_page(&result.url, Some(&result.page_id)).await?;
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "key": key,
            "title": result.title,
            "url": result.url,
        })),
    })
}

// ── Browser Screenshot ─────────────────────────────────────────────

async fn browser_screenshot(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let explicit_path = string_arg(&ctx.args, "path").or_else(|| string_arg(&ctx.args, "output"));
    let path = resolve_screenshot_path(explicit_path.as_deref());
    let bytes = ctx
        .session
        .capture_screenshot(&CaptureScreenshotOptions {
            full_page: boolean_arg(&ctx.args, "fullPage").unwrap_or(false),
            format: "png",
            quality: None,
        })
        .await?;
    let written = write_screenshot(&bytes, &path).await?;
    let title: String = ctx
        .session
        .evaluate(&EvaluateRequest::Expression {
            expression: "document.title".to_string(),
        })
        .await
        .map(|v| v.as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let url: String = ctx
        .session
        .evaluate(&EvaluateRequest::Expression {
            expression: "location.href".to_string(),
        })
        .await
        .map(|v| v.as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "path": written.path,
            "filename": written.filename,
            "title": title,
            "url": url,
        })),
    })
}

// ── Browser Scroll ─────────────────────────────────────────────────

async fn browser_scroll(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let direction = string_arg(&ctx.args, "direction").unwrap_or_else(|| "down".to_string());
    let amount = number_arg(&ctx.args, "amount").unwrap_or(300.0);
    let target = string_arg(&ctx.args, "ref").or_else(|| string_arg(&ctx.args, "selector"));

    let px = if direction == "down" || direction == "up" {
        0.0
    } else {
        amount
    };
    let py = if direction == "down" {
        amount
    } else if direction == "up" {
        -amount
    } else {
        0.0
    };

    if let Some(target) = &target {
        // Scroll a specific element.
        let resolved = resolve_target_from_args_optional(&ctx.args, &ctx.config)?;
        let selector = resolved
            .as_ref()
            .map(|r| r.selector.clone())
            .unwrap_or_else(|| target.clone());
        ctx.session
            .evaluate(&EvaluateRequest::Function {
                function_declaration: "function(sel, x, y) { var el = document.querySelector(sel); if (el) el.scrollBy({ left: x, top: y, behavior: 'smooth' }); }".to_string(),
                arguments: vec![json!(selector), json!(px), json!(py)],
            })
            .await?;
    } else {
        // Scroll the page.
        ctx.session
            .evaluate(&EvaluateRequest::Function {
                function_declaration:
                    "function(x, y) { window.scrollBy({ left: x, top: y, behavior: 'smooth' }); }"
                        .to_string(),
                arguments: vec![json!(px), json!(py)],
            })
            .await?;
    }

    let amount_out = js_number(amount);
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(json!({
            "scrolled": {
                "direction": direction,
                "amount": amount_out,
                "target": target.unwrap_or_else(|| "page".to_string()),
            }
        })),
    })
}

// ── Browser Console ────────────────────────────────────────────────

async fn browser_console(ctx: &mut SessionContext) -> Result<LocalToolResult, String> {
    let level = string_arg(&ctx.args, "level");
    let raw = ctx
        .session
        .evaluate(&EvaluateRequest::Expression {
            expression: "(globalThis.__futureConsoleLogs) || []".to_string(),
        })
        .await?;

    let logs: Vec<Value> = match raw {
        Value::Array(items) => items
            .into_iter()
            .filter(|e| e.is_object())
            .filter(|e| {
                level
                    .as_deref()
                    .map(|l| e.get("level").and_then(Value::as_str) == Some(l))
                    .unwrap_or(true)
            })
            .collect(),
        _ => Vec::new(),
    };

    let note = if logs.is_empty() {
        Some("No buffered console messages. The hook captures messages after a Future browser tool has touched the page.".to_string())
    } else {
        None
    };

    let mut sc = Map::new();
    sc.insert("logs".to_string(), Value::Array(logs));
    if let Some(note) = note {
        sc.insert("note".to_string(), Value::String(note));
    }
    Ok(LocalToolResult {
        text: None,
        structured_content: Some(Value::Object(sc)),
    })
}

// ── Helpers ─────────────────────────────────────────────────────────

fn resolve_target_from_args(
    args: &Map<String, Value>,
    config: &BrowserConfig,
) -> Result<ResolvedTarget, String> {
    let input = string_arg(args, "selector")
        .or_else(|| string_arg(args, "target"))
        .or_else(|| string_arg(args, "ref"));
    let Some(input) = input else {
        return Err("Expected ref, selector, or target.".to_string());
    };
    resolve_target(Some(&input), config).map_err(|e| e.to_string())
}

fn resolve_target_from_args_optional(
    args: &Map<String, Value>,
    config: &BrowserConfig,
) -> Result<Option<ResolvedTarget>, String> {
    let input = string_arg(args, "selector")
        .or_else(|| string_arg(args, "target"))
        .or_else(|| string_arg(args, "ref"));
    let Some(input) = input else {
        return Ok(None);
    };
    // If it's a ref but optional, just use as selector (for press on page body).
    match resolve_target(Some(&input), config) {
        Ok(t) => Ok(Some(t)),
        Err(_) => Ok(None),
    }
}

async fn save_active_page(url: &str, page_id: Option<&str>) -> Result<(), String> {
    let mut config = load_browser_config().await?;
    config.active_url = Some(url.to_string());
    if let Some(pid) = page_id {
        config.active_page_id = Some(pid.to_string());
    }
    save_browser_config(&config).await.map_err(String::from)
}

async fn clear_refs() -> Result<(), String> {
    let mut config = load_browser_config().await?;
    config.refs = Some(Map::new());
    save_browser_config(&config).await.map_err(String::from)
}

async fn save_refs_and_url(refs: &Map<String, Value>, url: &str) -> Result<(), String> {
    let mut config = load_browser_config().await?;
    config.refs = Some(refs.clone());
    config.active_url = Some(url.to_string());
    save_browser_config(&config).await.map_err(String::from)
}

/// `config.connection.endpoint || DEFAULT_ENDPOINT` — extracted so the
/// empty-endpoint fallback is unit-testable (validated config files always
/// carry an endpoint, so only a hand-built config reaches it).
fn config_endpoint_or_default(config: &BrowserConfig) -> String {
    let endpoint = config.connection.endpoint();
    if endpoint.is_empty() {
        DEFAULT_ENDPOINT.to_string()
    } else {
        endpoint.to_string()
    }
}

async fn endpoint_for(args: &Map<String, Value>) -> String {
    let config = load_browser_config().await.unwrap_or_default();
    string_arg(args, "endpoint").unwrap_or_else(|| config_endpoint_or_default(&config))
}

fn port_from_endpoint(endpoint: &str) -> Option<i64> {
    let stripped = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))?;
    let stripped = stripped.trim_end_matches('/');
    let (host, port) = stripped.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    port.parse::<i64>().ok().filter(|_| !port.is_empty())
}

async fn ensure_browser(args: &Map<String, Value>) -> Result<String, String> {
    let explicit_endpoint = string_arg(args, "endpoint");
    let endpoint = endpoint_for(args).await;
    if endpoint_reachable(&endpoint).await {
        return Ok(endpoint);
    }

    if let Some(explicit_endpoint) = explicit_endpoint {
        return Err(format!(
            "Local browser endpoint is not reachable: {explicit_endpoint}. Check the browser was started with a reachable remote debugging endpoint."
        ));
    }

    browser_start(args).await?;
    wait_for_saved_endpoint(DEFAULT_ENDPOINT, 10_000).await
}

/// The endpoint `browser_start` saved to the config, or the fallback when
/// the config carries none (defensive — validated configs always have one).
fn started_endpoint_or(config: &BrowserConfig, fallback_endpoint: &str) -> String {
    let ep = config.connection.endpoint();
    if ep.is_empty() {
        fallback_endpoint.to_string()
    } else {
        ep.to_string()
    }
}

async fn wait_for_saved_endpoint(
    fallback_endpoint: &str,
    timeout_ms: u64,
) -> Result<String, String> {
    let config = load_browser_config().await?;
    let started_endpoint = started_endpoint_or(&config, fallback_endpoint);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if endpoint_reachable(&started_endpoint).await {
            return Ok(started_endpoint);
        }
        crate::utils::time::sleep(250).await;
    }

    Err(format!(
        "Local browser endpoint is not reachable after auto-start: {started_endpoint}."
    ))
}

/// JS-style number serialization: JSON.stringify(300) === "300" (not "300.0").
fn js_number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 {
        Value::Number(serde_json::Number::from(v as i64))
    } else {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

// ── arg helpers ─────────────────────────────────────────────────────

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

fn boolean_arg(args: &Map<String, Value>, key: &str) -> Option<bool> {
    match args.get(key) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::backend::{
        InternalActionResult, InternalPageInfo, InternalTabInfo, InternalTabsResult,
        InternalTypeResult,
    };
    use crate::browser::browser_state::load_browser_config;

    // ── MockSession ───────────────────────────────────────────────────

    /// Canned BrowserSession for command-body tests: each method returns the
    /// configured result (or a sane default) and records the call.
    struct MockSession {
        tabs_new: Option<Result<InternalTabsResult, String>>,
        tabs_list: Option<Result<InternalTabsResult, String>>,
        tabs_select: Option<Result<InternalTabsResult, String>>,
        tabs_close: Option<Result<InternalTabsResult, String>>,
        open_result: Option<Result<InternalPageInfo, String>>,
        click_result: Option<Result<InternalActionResult, String>>,
        type_result: Option<Result<InternalTypeResult, String>>,
        press_result: Option<Result<InternalActionResult, String>>,
        screenshot_result: Option<Result<Vec<u8>, String>>,
        /// Custom evaluate responder (checked before the defaults).
        on_eval: Option<Box<dyn Fn(&EvaluateRequest) -> Result<Value, String> + Send + Sync>>,
        /// Snapshot function result (items etc.).
        snapshot_value: Value,
        /// Console-logs expression result.
        console_logs: Value,
        /// Recorded evaluate expressions (function source markers).
        eval_log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Default for MockSession {
        fn default() -> Self {
            MockSession {
                tabs_new: None,
                tabs_list: None,
                tabs_select: None,
                tabs_close: None,
                open_result: None,
                click_result: None,
                type_result: None,
                press_result: None,
                screenshot_result: None,
                on_eval: None,
                snapshot_value: json!({"title": "Snap T", "url": "http://snap/", "items": []}),
                console_logs: json!([]),
                eval_log: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    fn page_info(id: &str) -> InternalPageInfo {
        InternalPageInfo {
            page_id: id.to_string(),
            title: format!("Title {id}"),
            url: format!("http://{id}/"),
        }
    }

    fn action_result(id: &str) -> InternalActionResult {
        InternalActionResult {
            page_id: id.to_string(),
            title: format!("Title {id}"),
            url: format!("http://{id}/"),
            did_navigate: false,
        }
    }

    #[async_trait::async_trait]
    impl BrowserSession for MockSession {
        fn kind(&self) -> &'static str {
            "mock"
        }
        fn protocol(&self) -> &'static str {
            "cdp"
        }

        async fn open(
            &mut self,
            _url: &str,
            _options: OpenPageOptions,
        ) -> Result<InternalPageInfo, String> {
            match &self.open_result {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(page_info("p1")),
            }
        }

        async fn click(
            &mut self,
            _target: &ResolvedTarget,
            _options: ClickOptions,
        ) -> Result<InternalActionResult, String> {
            match &self.click_result {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(action_result("p1")),
            }
        }

        async fn r#type(
            &mut self,
            target: &ResolvedTarget,
            _text: &str,
            options: TypeOptions,
        ) -> Result<InternalTypeResult, String> {
            match &self.type_result {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(InternalTypeResult {
                    page_id: "p1".to_string(),
                    typed: target.selector.clone(),
                    submitted: options.submit.unwrap_or(false),
                }),
            }
        }

        async fn press(
            &mut self,
            _key: &str,
            _target: Option<&ResolvedTarget>,
            _options: PressOptions,
        ) -> Result<InternalActionResult, String> {
            match &self.press_result {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(action_result("p1")),
            }
        }

        async fn tabs(&mut self, action: &TabsAction) -> Result<InternalTabsResult, String> {
            let canned = match action {
                TabsAction::List => &self.tabs_list,
                TabsAction::New { .. } => &self.tabs_new,
                TabsAction::Select { .. } => &self.tabs_select,
                TabsAction::Close { .. } => &self.tabs_close,
            };
            match canned {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(match action {
                    TabsAction::List => InternalTabsResult::List { tabs: vec![] },
                    TabsAction::New { .. } => InternalTabsResult::New {
                        page: page_info("p-new"),
                        index: 0,
                    },
                    TabsAction::Select { .. } => InternalTabsResult::Select {
                        page: page_info("p-sel"),
                    },
                    TabsAction::Close { .. } => InternalTabsResult::Close {
                        url: "http://closed/".to_string(),
                        index: 0,
                    },
                }),
            }
        }

        async fn evaluate(&mut self, request: &EvaluateRequest) -> Result<Value, String> {
            let marker = match request {
                EvaluateRequest::Expression { expression } => expression.clone(),
                EvaluateRequest::Function {
                    function_declaration,
                    ..
                } => function_declaration.clone(),
            };
            self.eval_log.lock().unwrap().push(marker.clone());
            if let Some(f) = &self.on_eval {
                return f(request);
            }
            // Marker order matters: the snapshot source mentions
            // document.title/location.href in its body.
            if marker.contains("escapeCss") {
                return Ok(self.snapshot_value.clone());
            }
            if marker.contains("__futureConsoleLogs") {
                return Ok(self.console_logs.clone());
            }
            if marker.contains("document.title") {
                return Ok(json!("Eval Title"));
            }
            if marker.contains("location.href") {
                return Ok(json!("http://eval/"));
            }
            Ok(Value::Null)
        }

        async fn capture_screenshot(
            &mut self,
            _options: &CaptureScreenshotOptions,
        ) -> Result<Vec<u8>, String> {
            match &self.screenshot_result {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(b"png".to_vec()),
            }
        }

        async fn disconnect(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    // ── Test scaffolding ──────────────────────────────────────────────

    /// Isolated FUTURE_HOME (browser config) + the shared env lock.
    async fn isolated_home() -> (
        tokio::sync::MutexGuard<'static, ()>,
        crate::test_env::EnvGuard,
        tempfile::TempDir,
    ) {
        let guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        (guard, env, dir)
    }

    fn args(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn ctx_with(
        session: MockSession,
        config: BrowserConfig,
        a: Map<String, Value>,
    ) -> SessionContext {
        SessionContext {
            session: Box::new(session),
            config,
            args: a,
        }
    }

    fn structured(result: &LocalToolResult) -> Value {
        result.structured_content.clone().unwrap_or(Value::Null)
    }

    // ── Catalog / dispatch ────────────────────────────────────────────

    #[test]
    fn catalog_and_predicate() {
        let catalog = browser_tool_catalog();
        assert_eq!(catalog.len(), 1);
        let (name, entry) = &catalog[0];
        assert_eq!(*name, "browser");
        assert!(entry.description.contains("browser"));
        assert!(entry.args.iter().any(|(k, _)| *k == "command"));
        assert!(entry.example.contains("open"));
        assert!(is_browser_tool("browser"));
        assert!(!is_browser_tool("web_search"));
    }

    #[tokio::test]
    async fn dispatch_requires_command_and_rejects_unknown() {
        let (out, _cap) = Output::memory();
        let err = call_browser_tool("browser", &Map::new(), &out)
            .await
            .unwrap_err();
        assert_eq!(err, "browser tool requires \"command\" argument.");

        let err = call_browser_tool("browser", &args(&[("command", json!("bogus"))]), &out)
            .await
            .unwrap_err();
        assert!(err.contains("Unknown browser command: \"bogus\""), "{err}");
        assert!(err.contains("start, status, tabs, open"), "{err}");
    }

    // ── tabs ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn tabs_list_and_default_action() {
        let (_g, _e, _d) = isolated_home().await;
        let mut session = MockSession::default();
        session.tabs_list = Some(Ok(InternalTabsResult::List {
            tabs: vec![
                InternalTabInfo {
                    page_id: "a".to_string(),
                    index: 0,
                    title: "TA".to_string(),
                    url: "http://a/".to_string(),
                    active: true,
                },
                InternalTabInfo {
                    page_id: "b".to_string(),
                    index: 1,
                    title: "TB".to_string(),
                    url: "http://b/".to_string(),
                    active: false,
                },
            ],
        }));
        // No "action" arg → defaults to list.
        let mut ctx = ctx_with(session, BrowserConfig::default(), Map::new());
        let result = browser_tabs(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["tabCount"], json!(2));
        assert_eq!(sc["tabs"][0]["title"], json!("TA"));
        assert_eq!(sc["tabs"][0]["active"], json!(true));
        assert_eq!(sc["tabs"][1]["active"], json!(false));
    }

    #[tokio::test]
    async fn tabs_list_unexpected_variant_yields_empty() {
        let (_g, _e, _d) = isolated_home().await;
        let mut session = MockSession::default();
        // List answered with a non-List variant → empty tabs vec.
        session.tabs_list = Some(Ok(InternalTabsResult::Close {
            url: "u".to_string(),
            index: 0,
        }));
        let mut ctx = ctx_with(session, BrowserConfig::default(), Map::new());
        let result = browser_tabs(&mut ctx).await.unwrap();
        assert_eq!(structured(&result)["tabCount"], json!(0));
    }

    #[tokio::test]
    async fn tabs_new_select_close_happy_paths() {
        let (_g, _e, _d) = isolated_home().await;
        let mut ctx = ctx_with(
            MockSession {
                tabs_new: Some(Ok(InternalTabsResult::New {
                    page: page_info("p9"),
                    index: 2,
                })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("new"))]),
        );
        let result = browser_tabs(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["created"]["index"], json!(2));
        assert_eq!(sc["created"]["url"], json!("http://p9/"));
        // Active page persisted to config.
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.active_page_id.as_deref(), Some("p9"));
        assert_eq!(saved.active_url.as_deref(), Some("http://p9/"));

        let mut ctx = ctx_with(
            MockSession {
                tabs_select: Some(Ok(InternalTabsResult::Select {
                    page: page_info("p7"),
                })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("select")), ("index", json!(1))]),
        );
        let result = browser_tabs(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["selected"]["index"], json!(1));
        assert_eq!(sc["selected"]["url"], json!("http://p7/"));

        let mut ctx = ctx_with(
            MockSession {
                tabs_close: Some(Ok(InternalTabsResult::Close {
                    url: "http://gone/".to_string(),
                    index: 1,
                })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("close")), ("index", json!(1))]),
        );
        let result = browser_tabs(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["closed"]["index"], json!(1));
        assert_eq!(sc["closed"]["url"], json!("http://gone/"));
    }

    #[tokio::test]
    async fn tabs_error_paths() {
        let (_g, _e, _d) = isolated_home().await;
        // Unknown action WITHOUT an index hits the index validation first.
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("action", json!("sideways"))]),
        );
        let err = browser_tabs(&mut ctx).await.unwrap_err();
        assert!(err.contains("requires a valid 0-based index"), "{err}");
        // Unknown action WITH an index reaches the action validation.
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("action", json!("sideways")), ("index", json!(0))]),
        );
        let err = browser_tabs(&mut ctx).await.unwrap_err();
        assert!(
            err.contains("action must be \"list\", \"new\", \"select\", or \"close\""),
            "{err}"
        );

        // select/close without index.
        for action in ["select", "close"] {
            let mut ctx = ctx_with(
                MockSession::default(),
                BrowserConfig::default(),
                args(&[("action", json!(action))]),
            );
            let err = browser_tabs(&mut ctx).await.unwrap_err();
            assert!(err.contains("requires a valid 0-based index"), "{err}");
            // Negative index.
            let mut ctx = ctx_with(
                MockSession::default(),
                BrowserConfig::default(),
                args(&[("action", json!(action)), ("index", json!(-1))]),
            );
            let err = browser_tabs(&mut ctx).await.unwrap_err();
            assert!(err.contains("requires a valid 0-based index"), "{err}");
        }

        // Mismatched result variants → "Unexpected tabs result".
        let mut ctx = ctx_with(
            MockSession {
                tabs_new: Some(Ok(InternalTabsResult::List { tabs: vec![] })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("new"))]),
        );
        assert_eq!(
            browser_tabs(&mut ctx).await.unwrap_err(),
            "Unexpected tabs result"
        );

        let mut ctx = ctx_with(
            MockSession {
                tabs_select: Some(Ok(InternalTabsResult::List { tabs: vec![] })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("select")), ("index", json!(0))]),
        );
        assert_eq!(
            browser_tabs(&mut ctx).await.unwrap_err(),
            "Unexpected tabs result"
        );

        let mut ctx = ctx_with(
            MockSession {
                tabs_close: Some(Ok(InternalTabsResult::List { tabs: vec![] })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("close")), ("index", json!(0))]),
        );
        assert_eq!(
            browser_tabs(&mut ctx).await.unwrap_err(),
            "Unexpected tabs result"
        );
    }

    // ── open ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn open_requires_url_and_saves_active_page() {
        let (_g, _e, _d) = isolated_home().await;
        let mut ctx = ctx_with(MockSession::default(), BrowserConfig::default(), Map::new());
        let err = browser_open(&mut ctx).await.unwrap_err();
        assert_eq!(err, "browser command open requires url.");

        // Seed refs so we can verify open() clears them.
        let mut seeded = BrowserConfig {
            version: 2,
            ..Default::default()
        };
        seeded.refs = Some(args(&[("a1", json!("#x"))]));
        save_browser_config(&seeded).await.unwrap();

        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("url", json!("http://dest/"))]),
        );
        let result = browser_open(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["title"], json!("Title p1"));
        assert_eq!(sc["url"], json!("http://p1/"));
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.refs.unwrap().len(), 0);
        assert_eq!(saved.active_page_id.as_deref(), Some("p1"));
    }

    // ── snapshot ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn snapshot_formats_lines_refs_and_states() {
        let (_g, _e, _d) = isolated_home().await;
        let mut session = MockSession::default();
        session.snapshot_value = json!({
            "title": "Snap T", "url": "http://snap/",
            "items": [
                {"ref": "b1", "selector": "#go", "role": "button", "name": "Go",
                 "tag": "button", "disabled": true, "checked": null, "href": null},
                {"ref": "c1", "selector": "#chk", "role": "checkbox", "name": "Agree",
                 "tag": "input", "disabled": false, "checked": true, "href": null},
                {"ref": "a1", "selector": "a.docs", "role": "link", "name": "Docs",
                 "tag": "a", "disabled": false, "checked": null, "href": "https://d/"},
                {"ref": "t4", "selector": "p", "role": "text", "name": "hello"},
            ],
        });
        let mut ctx = ctx_with(session, BrowserConfig::default(), Map::new());
        let result = browser_snapshot(&mut ctx).await.unwrap();
        let text = result.text.as_ref().expect("text");
        assert!(
            text.starts_with("Page: Snap T\nURL: http://snap/\n\n"),
            "{text}"
        );
        assert!(text.contains("- button \"Go\" [ref=b1] disabled"), "{text}");
        assert!(
            text.contains("- checkbox \"Agree\" [ref=c1] checked=true"),
            "{text}"
        );
        assert!(
            text.contains("- link \"Docs\" [ref=a1] href=https://d/"),
            "{text}"
        );
        assert!(text.contains("- text \"hello\" [ref=t4]"), "{text}");
        let sc = structured(&result);
        let elements = sc["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 4);
        assert_eq!(elements[1]["checked"], json!(true));
        assert_eq!(elements[2]["href"], json!("https://d/"));
        assert_eq!(elements[3]["tag"], Value::Null);
        // Refs persisted for later click-by-ref.
        let saved = load_browser_config().await.unwrap();
        let refs = saved.refs.unwrap();
        assert_eq!(refs.get("b1").and_then(Value::as_str), Some("#go"));
        assert_eq!(saved.active_url.as_deref(), Some("http://snap/"));
    }

    #[tokio::test]
    async fn snapshot_limit_arg_is_serialized_js_style() {
        let (_g, _e, _d) = isolated_home().await;
        let session = MockSession::default();
        let log = session.eval_log.clone();
        let mut ctx = ctx_with(
            session,
            BrowserConfig::default(),
            args(&[("limit", json!(5))]),
        );
        browser_snapshot(&mut ctx).await.unwrap();
        // The snapshot function ran (limit serialized as an integer).
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    // ── click ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn click_requires_a_target_and_reports_result() {
        let (_g, _e, _d) = isolated_home().await;
        let mut ctx = ctx_with(MockSession::default(), BrowserConfig::default(), Map::new());
        let err = browser_click(&mut ctx).await.unwrap_err();
        assert_eq!(err, "Expected ref, selector, or target.");

        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("selector", json!("#go"))]),
        );
        let result = browser_click(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["clicked"], json!("#go"));
        assert_eq!(sc["selector"], json!("#go"));
        assert_eq!(sc["title"], json!("Title p1"));
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.active_page_id.as_deref(), Some("p1"));
    }

    #[tokio::test]
    async fn click_by_ref_uses_saved_refs() {
        let (_g, _e, _d) = isolated_home().await;
        let mut config = BrowserConfig::default();
        config.refs = Some(args(&[("b1", json!("#saved-sel"))]));
        let mut ctx = ctx_with(
            MockSession::default(),
            config,
            args(&[("ref", json!("b1"))]),
        );
        let result = browser_click(&mut ctx).await.unwrap();
        assert_eq!(structured(&result)["selector"], json!("#saved-sel"));
    }

    // ── type ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn type_requires_text_and_reports_submitted() {
        let (_g, _e, _d) = isolated_home().await;
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("selector", json!("#in"))]),
        );
        let err = browser_type(&mut ctx).await.unwrap_err();
        assert_eq!(err, "browser command type requires text.");

        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[
                ("selector", json!("#in")),
                ("text", json!("hello")),
                ("submit", json!(true)),
                ("clear", json!(false)),
            ]),
        );
        let result = browser_type(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["typed"], json!("#in"));
        assert_eq!(sc["submitted"], json!(true));
    }

    // ── press ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn press_requires_key_and_allows_page_level() {
        let (_g, _e, _d) = isolated_home().await;
        let mut ctx = ctx_with(MockSession::default(), BrowserConfig::default(), Map::new());
        let err = browser_press(&mut ctx).await.unwrap_err();
        assert_eq!(err, "browser command press requires key.");

        // No target → page-level press.
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("key", json!("Enter"))]),
        );
        let result = browser_press(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["key"], json!("Enter"));
        assert_eq!(sc["url"], json!("http://p1/"));

        // With a target selector.
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("key", json!("Tab")), ("selector", json!("#f"))]),
        );
        browser_press(&mut ctx).await.unwrap();
    }

    // ── screenshot ────────────────────────────────────────────────────

    #[tokio::test]
    async fn screenshot_writes_file_and_reports_metadata() {
        let (_g, _e, dir) = isolated_home().await;
        let explicit = dir.path().join("shot.png");
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("path", json!(explicit.to_string_lossy()))]),
        );
        let result = browser_screenshot(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["title"], json!("Eval Title"));
        assert_eq!(sc["url"], json!("http://eval/"));
        assert_eq!(sc["filename"], json!("shot.png"));
        assert!(explicit.exists());

        // "output" alias + default path under FUTURE_HOME artifacts.
        let mut ctx = ctx_with(
            MockSession::default(),
            BrowserConfig::default(),
            args(&[("output", json!(explicit.to_string_lossy()))]),
        );
        browser_screenshot(&mut ctx).await.unwrap();
        let mut ctx = ctx_with(MockSession::default(), BrowserConfig::default(), Map::new());
        let result = browser_screenshot(&mut ctx).await.unwrap();
        let path = structured(&result)["path"].as_str().unwrap().to_string();
        assert!(path.contains("artifacts"), "{path}");
    }

    // ── scroll ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn scroll_directions_and_targets() {
        let (_g, _e, _d) = isolated_home().await;
        // Page-level scroll: down/up/left/right.
        for direction in ["down", "up", "left", "right"] {
            let session = MockSession::default();
            let log = session.eval_log.clone();
            let mut ctx = ctx_with(
                session,
                BrowserConfig::default(),
                args(&[("direction", json!(direction))]),
            );
            let result = browser_scroll(&mut ctx).await.unwrap();
            let sc = structured(&result);
            assert_eq!(sc["scrolled"]["direction"], json!(direction));
            assert_eq!(sc["scrolled"]["amount"], json!(300));
            assert_eq!(sc["scrolled"]["target"], json!("page"));
            // The page-scroll function (window.scrollBy) ran.
            let calls = log.lock().unwrap().clone();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].contains("window.scrollBy"), "{:?}", calls[0]);
        }

        // Element scroll with explicit amount.
        let session = MockSession::default();
        let log = session.eval_log.clone();
        let mut ctx = ctx_with(
            session,
            BrowserConfig::default(),
            args(&[
                ("direction", json!("up")),
                ("amount", json!(50)),
                ("selector", json!("#pane")),
            ]),
        );
        let result = browser_scroll(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["scrolled"]["target"], json!("#pane"));
        assert_eq!(sc["scrolled"]["amount"], json!(50));
        let calls = log.lock().unwrap().clone();
        assert!(
            calls[0].contains("document.querySelector"),
            "{:?}",
            calls[0]
        );
    }

    // ── console ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn console_filters_by_level_and_notes_empty() {
        let (_g, _e, _d) = isolated_home().await;
        let mut session = MockSession::default();
        session.console_logs = json!([
            {"level": "log", "text": "hi", "time": "t1"},
            {"level": "error", "text": "boom", "time": "t2"},
            "not-an-object",
        ]);
        let mut ctx = ctx_with(session, BrowserConfig::default(), Map::new());
        let result = browser_console(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["logs"].as_array().unwrap().len(), 2);
        assert!(sc.get("note").is_none());

        // Level filter.
        let mut session = MockSession::default();
        session.console_logs = json!([
            {"level": "log", "text": "hi", "time": "t1"},
            {"level": "error", "text": "boom", "time": "t2"},
        ]);
        let mut ctx = ctx_with(
            session,
            BrowserConfig::default(),
            args(&[("level", json!("error"))]),
        );
        let result = browser_console(&mut ctx).await.unwrap();
        let logs = structured(&result)["logs"].as_array().unwrap().clone();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["text"], json!("boom"));

        // Empty → explanatory note.
        let mut ctx = ctx_with(MockSession::default(), BrowserConfig::default(), Map::new());
        let result = browser_console(&mut ctx).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["logs"].as_array().unwrap().len(), 0);
        assert!(sc["note"]
            .as_str()
            .unwrap()
            .contains("No buffered console messages"));

        // Non-array value → empty + note.
        let mut session = MockSession::default();
        session.console_logs = json!("junk");
        let mut ctx = ctx_with(session, BrowserConfig::default(), Map::new());
        let result = browser_console(&mut ctx).await.unwrap();
        assert!(structured(&result).get("note").is_some());
    }

    // ── helpers ───────────────────────────────────────────────────────

    #[test]
    fn arg_helper_edge_shapes() {
        let a = args(&[
            ("empty", json!("")),
            ("s", json!("v")),
            ("n", json!(2.5)),
            ("ns", json!("3")),
            ("b", json!(true)),
            ("bn", json!(1)),
        ]);
        assert_eq!(string_arg(&a, "empty"), None);
        assert_eq!(string_arg(&a, "s"), Some("v".to_string()));
        assert_eq!(string_arg(&a, "missing"), None);
        assert_eq!(number_arg(&a, "n"), Some(2.5));
        assert_eq!(number_arg(&a, "ns"), None);
        assert_eq!(boolean_arg(&a, "b"), Some(true));
        assert_eq!(boolean_arg(&a, "bn"), None);
    }

    #[test]
    fn js_number_serialization() {
        assert_eq!(js_number(300.0), json!(300));
        assert_eq!(js_number(0.5), json!(0.5));
        assert_eq!(js_number(-4.0), json!(-4));
        // Beyond the safe-integer range → float fallback.
        let big = js_number(9_007_199_254_740_993.0);
        assert!(big.is_f64() || big.is_i64() || big.is_u64());
        // Non-finite → from_f64 fails → Null.
        assert_eq!(js_number(f64::NAN), Value::Null);
    }

    #[test]
    fn port_from_endpoint_variants() {
        assert_eq!(port_from_endpoint("http://127.0.0.1:9222"), Some(9222));
        assert_eq!(port_from_endpoint("https://h:443/"), Some(443));
        assert_eq!(port_from_endpoint("http://:80"), None);
        assert_eq!(port_from_endpoint("ftp://h:21"), None);
        assert_eq!(port_from_endpoint("http://host"), None);
        assert_eq!(port_from_endpoint("http://h:abc"), None);
        assert_eq!(port_from_endpoint("http://h:"), None);
    }

    #[tokio::test]
    async fn endpoint_for_resolution_order() {
        let (_g, _e, _d) = isolated_home().await;
        // Args win.
        let a = args(&[("endpoint", json!("http://arg/"))]);
        assert_eq!(endpoint_for(&a).await, "http://arg/");
        // Config endpoint next.
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: "http://cfg/".to_string(),
            },
            ..Default::default()
        };
        save_browser_config(&config).await.unwrap();
        assert_eq!(endpoint_for(&Map::new()).await, "http://cfg/");
    }

    #[tokio::test]
    async fn endpoint_for_default_when_config_absent() {
        let (_g, _e, _d) = isolated_home().await;
        assert_eq!(endpoint_for(&Map::new()).await, DEFAULT_ENDPOINT);
    }

    #[tokio::test]
    async fn wait_for_saved_endpoint_timeout_and_reachable() {
        let (_g, _e, _d) = isolated_home().await;
        // Nothing reachable at the saved endpoint → bounded timeout error.
        let err = wait_for_saved_endpoint("http://127.0.0.1:1", 50)
            .await
            .unwrap_err();
        assert!(err.contains("not reachable after auto-start"), "{err}");

        // Saved endpoint reachable → returned immediately.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/json/version",
            200,
            "{}",
        )])
        .await;
        save_browser_config(&BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: base.clone(),
            },
            ..Default::default()
        })
        .await
        .unwrap();
        let ep = wait_for_saved_endpoint("http://unused/", 2_000)
            .await
            .unwrap();
        assert_eq!(ep, base);
    }

    #[test]
    fn started_endpoint_or_fallback_for_empty_config_endpoint() {
        // Validated configs always carry an endpoint, but a hand-built one
        // can be empty → the fallback kicks in.
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: String::new(),
            },
            ..Default::default()
        };
        assert_eq!(started_endpoint_or(&config, "http://fb/"), "http://fb/");
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: "http://real/".to_string(),
            },
            ..Default::default()
        };
        assert_eq!(started_endpoint_or(&config, "http://fb/"), "http://real/");
    }

    #[tokio::test]
    async fn ensure_browser_rejects_unreachable_explicit_endpoint() {
        let (_g, _e, _d) = isolated_home().await;
        let a = args(&[("endpoint", json!("http://127.0.0.1:1"))]);
        let err = ensure_browser(&a).await.unwrap_err();
        assert!(
            err.contains("Local browser endpoint is not reachable"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn resolve_target_optional_falls_back_on_bad_ref() {
        let config = BrowserConfig::default();
        // A ref-shaped input with no saved refs → None (not an error).
        let a = args(&[("ref", json!("a1"))]);
        assert!(resolve_target_from_args_optional(&a, &config)
            .unwrap()
            .is_none());
        // Direct selector resolves.
        let a = args(&[("selector", json!("#x"))]);
        let t = resolve_target_from_args_optional(&a, &config)
            .unwrap()
            .unwrap();
        assert_eq!(t.selector, "#x");
        // Nothing at all → None.
        assert!(resolve_target_from_args_optional(&Map::new(), &config)
            .unwrap()
            .is_none());
    }

    // ── status ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn status_reachable_http_error_and_unreachable() {
        let (_g, _e, _d) = isolated_home().await;
        // Reachable with a version payload.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/json/version",
            200,
            r#"{"Browser":"Chrome/126"}"#,
        )])
        .await;
        let a = args(&[("endpoint", json!(base))]);
        let result = browser_status(&a).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["reachable"], json!(true));
        assert_eq!(sc["version"]["Browser"], json!("Chrome/126"));
        assert_eq!(sc["endpoint"], json!(base));

        // HTTP error status.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/json/version",
            500,
            "{}",
        )])
        .await;
        let a = args(&[("endpoint", json!(base))]);
        let result = browser_status(&a).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["reachable"], json!(false));
        assert_eq!(sc["status"], json!(500));

        // Connection refused.
        let a = args(&[("endpoint", json!("http://127.0.0.1:1"))]);
        let result = browser_status(&a).await.unwrap();
        let sc = structured(&result);
        assert_eq!(sc["reachable"], json!(false));
        assert_eq!(
            sc["error"],
            json!("Local browser endpoint is not reachable.")
        );
    }

    // ── is_permission_error ───────────────────────────────────────────

    #[test]
    fn permission_error_marker_match() {
        assert!(is_permission_error("Safari remote automation is disabled."));
        assert!(!is_permission_error("something else"));
    }

    // ── with_session integration (mock CDP) ───────────────────────────

    /// Save a v2 CDP config pointing at the mock browser.
    async fn save_cdp_config(endpoint: &str, browser_kind: &str) {
        save_browser_config(&BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: browser_kind.to_string(),
                endpoint: endpoint.to_string(),
            },
            ..Default::default()
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_session_full_cdp_roundtrip_and_kind_refinement() {
        let (_g, _e, _d) = isolated_home().await;
        let mock = crate::test_cdp::MockCdp::start().await;
        save_cdp_config(&mock.http_url, "chromium").await;

        let (out, _cap) = Output::memory();
        let result = call_browser_tool(
            "browser",
            &args(&[("command", json!("tabs")), ("action", json!("list"))]),
            &out,
        )
        .await
        .unwrap();
        let sc = structured(&result);
        // The mock browser has one initial page target.
        assert_eq!(sc["tabCount"], json!(1));
        // browserKind "chromium" was refined to "chrome" and persisted.
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.connection.browser_kind(), "chrome");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_session_webdriver_config_creates_safari_session() {
        let (_g, _e, _d) = isolated_home().await;
        // WebDriver config + a mock that answers /json/version (for
        // endpoint_reachable) and the webdriver calls (for tabs list).
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json("/json/version", 200, "{}"),
            crate::test_server::HttpRoute::json(
                "/session/s1/window/handles",
                200,
                r#"{"value":["h1"]}"#,
            ),
            crate::test_server::HttpRoute::json("/session/s1/window", 200, r#"{"value":"h1"}"#),
            crate::test_server::HttpRoute::json("/session/s1/title", 200, r#"{"value":"T"}"#),
            crate::test_server::HttpRoute::json("/session/s1/url", 200, r#"{"value":"u"}"#),
        ])
        .await;
        save_browser_config(&BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: base,
                session_id: "s1".to_string(),
                driver_pid: None,
            },
            ..Default::default()
        })
        .await
        .unwrap();

        let (out, _cap) = Output::memory();
        let result = call_browser_tool("browser", &args(&[("command", json!("tabs"))]), &out)
            .await
            .unwrap();
        assert_eq!(structured(&result)["tabCount"], json!(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_session_explicit_endpoint_error_surfaces() {
        let (_g, _e, _d) = isolated_home().await;
        // Explicit endpoint arg + unreachable → ensure_browser error
        // propagates without any auto-start attempt.
        let (out, _cap) = Output::memory();
        let err = call_browser_tool(
            "browser",
            &args(&[
                ("command", json!("tabs")),
                ("endpoint", json!("http://127.0.0.1:1")),
            ]),
            &out,
        )
        .await
        .unwrap_err();
        assert!(err.contains("not reachable"), "{err}");
    }

    // ── browser_start ─────────────────────────────────────────────────

    /// Serve `GET /json/version` on a FIXED port with a hand-rolled HTTP
    /// responder (spawn_http only binds ephemeral ports).
    async fn serve_json_version_on(port: i64, ws_url: &str) -> tokio::task::JoinHandle<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port as u16))
            .await
            .expect("bind fixed port");
        let body = format!(r#"{{"Browser":"Chrome/126.0.0.0","webSocketDebuggerUrl":"{ws_url}"}}"#);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        })
    }

    /// Find a currently-free port.
    fn free_port() -> i64 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        drop(listener);
        port
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_already_running_notes_and_config_update() {
        let (_g, _e, _d) = isolated_home().await;
        let port = free_port();
        let _server = serve_json_version_on(port, "ws://127.0.0.1:1/ws").await;
        let endpoint = format!("http://127.0.0.1:{port}");

        // First call: existing endpoint (default 9222) differs → update note.
        let result = browser_start(&args(&[("port", json!(port))]))
            .await
            .unwrap();
        let sc = structured(&result);
        assert_eq!(sc["status"], json!("already_running"));
        assert_eq!(sc["endpoint"], json!(endpoint));
        assert!(sc["note"].as_str().unwrap().contains("was updated"), "{sc}");

        // Second call: config now points at the same endpoint → plain note.
        let result = browser_start(&args(&[("port", json!(port))]))
            .await
            .unwrap();
        let sc = structured(&result);
        assert_eq!(
            sc["note"],
            json!("Browser is already running at this endpoint.")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_without_any_browser_binary_errors() {
        let (_g, _e, _d) = isolated_home().await;
        // Force the no-launcher outcome (this dev machine has a real Chrome,
        // so platform discovery would otherwise always succeed).
        *BROWSER_LAUNCHER_OVERRIDE.lock().unwrap() = Some(None);
        let port = free_port();
        let err = browser_start(&args(&[("port", json!(port))]))
            .await
            .unwrap_err();
        *BROWSER_LAUNCHER_OVERRIDE.lock().unwrap() = None;
        assert!(err.contains("Could not find Chrome or Edge"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn start_launch_becomes_reachable_reports_started() {
        let (_g, _e, dir) = isolated_home().await;
        // Fake chrome: a python script serving /json/version on the port
        // given via --remote-debugging-port.
        let py = dir.path().join("fake_chrome.py");
        std::fs::write(
            &py,
            r#"import http.server, socketserver, sys, json, time
port = 0
for a in sys.argv:
    if a.startswith("--remote-debugging-port="):
        port = int(a.split("=")[1])
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({"Browser": "FakeChrome/1", "webSocketDebuggerUrl": "ws://127.0.0.1:1/ws"}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass
time.sleep(0.3)
socketserver.TCPServer(("127.0.0.1", port), H).serve_forever()
"#,
        )
        .unwrap();
        let sh = dir.path().join("chrome");
        std::fs::write(
            &sh,
            format!("#!/bin/sh\nexec python3 {} \"$@\"\n", py.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let port = free_port();
        let result = browser_start(&args(&[
            ("port", json!(port)),
            ("executablePath", json!(sh.to_string_lossy().to_string())),
            ("url", json!("http://home/")),
        ]))
        .await
        .unwrap();
        let sc = structured(&result);
        assert_eq!(sc["status"], json!("started"));
        assert_eq!(sc["port"], json!(port));
        assert_eq!(sc["requestedPort"], json!(port));
        assert!(
            sc["profileDir"].as_str().unwrap().contains("profile"),
            "{sc}"
        );
        // Config saved with the active url.
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.active_url.as_deref(), Some("http://home/"));
        assert_eq!(
            saved.connection.endpoint(),
            format!("http://127.0.0.1:{port}")
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn start_launch_never_reachable_reports_starting() {
        let (_g, _e, dir) = isolated_home().await;
        // Occupy the requested port with a NON-HTTP listener → resolve_port
        // scans to a free port; /bin/true exits immediately → the endpoint
        // never comes up → 10 s wait → "starting".
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = holder.local_addr().unwrap().port() as i64;
        let profile = dir.path().join("custom-profile");
        let result = browser_start(&args(&[
            ("port", json!(taken)),
            ("executablePath", json!("/bin/true")),
            ("profileDir", json!(profile.to_string_lossy())),
        ]))
        .await
        .unwrap();
        drop(holder);
        let sc = structured(&result);
        assert_eq!(sc["status"], json!("starting"));
        assert!(
            sc["note"].as_str().unwrap().contains("did not answer"),
            "{sc}"
        );
        // The scanned port differs from the occupied requested one.
        assert_ne!(sc["port"], json!(taken));
        assert_eq!(sc["requestedPort"], json!(taken));
        // Explicit profileDir is honored.
        assert_eq!(
            sc["profileDir"],
            json!(profile.to_string_lossy().to_string())
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn start_default_profile_dir_uses_scanned_port() {
        let (_g, _e, _d) = isolated_home().await;
        // Occupied requested port + no explicit profileDir → profile-N dir.
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = holder.local_addr().unwrap().port() as i64;
        let result = browser_start(&args(&[
            ("port", json!(taken)),
            ("executablePath", json!("/bin/true")),
        ]))
        .await
        .unwrap();
        drop(holder);
        let sc = structured(&result);
        let resolved = sc["port"].as_i64().unwrap();
        assert_ne!(resolved, taken);
        assert!(
            sc["profileDir"]
                .as_str()
                .unwrap()
                .ends_with(&format!("profile-{resolved}")),
            "{sc}"
        );
    }

    // ── Safari start path ─────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn start_safari_already_running_persists_config() {
        let (_g, _e, _d) = isolated_home().await;
        // Mock safaridriver: /status + session creation.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json("/status", 200, r#"{"ready":true}"#),
            crate::test_server::HttpRoute::json(
                "/session",
                200,
                r#"{"sessionId":"sid-9","value":{}}"#,
            ),
        ])
        .await;
        let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        let result = browser_start(&args(&[
            ("browser", json!("safari")),
            ("port", json!(port)),
            ("url", json!("http://safari/")),
        ]))
        .await
        .unwrap();
        let sc = structured(&result);
        assert_eq!(sc["status"], json!("already_running"));
        assert_eq!(sc["browserKind"], json!("safari"));
        assert_eq!(sc["port"], json!(port));
        // The webdriver connection config was persisted.
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.connection.protocol(), "webdriver");
        assert_eq!(saved.connection.session_id(), Some("sid-9"));
        assert_eq!(saved.active_url.as_deref(), Some("http://safari/"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_safari_permission_error_is_actionable() {
        let (_g, _e, _d) = isolated_home().await;
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json("/status", 200, r#"{"ready":true}"#),
            crate::test_server::HttpRoute::json(
                "/session",
                500,
                r#"{"value":{"error":"session not created","message":"Allow Remote Automation"}}"#,
            ),
        ])
        .await;
        let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        let result = browser_start(&args(&[
            ("browser", json!("safari")),
            ("port", json!(port)),
        ]))
        .await
        .unwrap();
        let sc = structured(&result);
        assert_eq!(sc["status"], json!("permission_required"));
        assert_eq!(
            sc["actionRequired"]["command"],
            json!("safaridriver --enable")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_safari_other_error_propagates() {
        let (_g, _e, _d) = isolated_home().await;
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json("/status", 200, r#"{"ready":true}"#),
            crate::test_server::HttpRoute::json(
                "/session",
                500,
                r#"{"value":{"error":"unknown error","message":"weird failure"}}"#,
            ),
        ])
        .await;
        let port: i64 = base.rsplit(':').next().unwrap().parse().unwrap();
        let err = browser_start(&args(&[
            ("browser", json!("safari")),
            ("port", json!(port)),
        ]))
        .await
        .unwrap_err();
        assert!(err.contains("weird failure"), "{err}");
    }

    // ── with_session retry + ensure_browser auto-start ────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn create_session_webdriver_requires_session_id() {
        // Hand-built config: the empty session id fails before any network.
        // (File-backed configs can never reach this: a blank sessionId fails
        // validation, a missing one recovers to the default CDP config.)
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: "http://x".to_string(),
                session_id: String::new(),
                driver_pid: None,
            },
            ..Default::default()
        };
        let err = create_session(&config, "http://x").await.err().unwrap();
        assert_eq!(err, "sessionId required for webdriver");

        // A valid webdriver config builds a Safari session.
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Webdriver {
                browser_kind: "safari".to_string(),
                endpoint: "http://x".to_string(),
                session_id: "s1".to_string(),
                driver_pid: None,
            },
            ..Default::default()
        };
        let session = create_session(&config, "http://x").await.unwrap();
        assert_eq!(session.kind(), "safari");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_browser_auto_start_success() {
        let (_g, _e, _d) = isolated_home().await;
        // Needs the default 9222 port; skip when the user's own browser
        // holds it.
        if std::net::TcpListener::bind("127.0.0.1:9222").is_err() {
            return;
        }
        let _server = serve_json_version_on(9222, "ws://127.0.0.1:1/ws").await;
        // No explicit endpoint; config endpoint unreachable-at-first is not
        // required here: the DEFAULT endpoint is what ensure_browser checks.
        let endpoint = ensure_browser(&Map::new()).await.unwrap();
        assert_eq!(endpoint, DEFAULT_ENDPOINT);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_browser_auto_start_failure_propagates() {
        let (_g, _e, _d) = isolated_home().await;
        if std::net::TcpListener::bind("127.0.0.1:9222").is_err() {
            return;
        }
        // Nothing on 9222 and no browser binary → browser_start fails.
        *BROWSER_LAUNCHER_OVERRIDE.lock().unwrap() = Some(None);
        let err = ensure_browser(&Map::new()).await.unwrap_err();
        *BROWSER_LAUNCHER_OVERRIDE.lock().unwrap() = None;
        assert!(err.contains("Could not find Chrome or Edge"), "{err}");
    }

    // ── create_session refinement arms ────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn create_session_refine_failure_keeps_chromium() {
        let (_g, _e, _d) = isolated_home().await;
        // /json/version answers ONCE (for ensure_browser), then fails (the
        // refinement probe) → browserKind stays "chromium".
        let mock = crate::test_cdp::MockCdp::start().await;
        let version_ok = format!(
            r#"{{"Browser":"Chrome/126","webSocketDebuggerUrl":"{}"}}"#,
            mock.ws_url
        );
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sequence(
            "/json/version",
            vec![
                // 1: ensure_browser reachability probe.
                (200, &version_ok),
                // 2: create_session's refinement probe → fails → keep
                //    "chromium".
                (500, "{}"),
                // 3: the session's own init resolve (must succeed).
                (200, &version_ok),
            ],
        )])
        .await;
        save_cdp_config(&base, "chromium").await;
        let (out, _cap) = Output::memory();
        let result = call_browser_tool(
            "browser",
            &args(&[("command", json!("tabs")), ("action", json!("list"))]),
            &out,
        )
        .await
        .unwrap();
        assert_eq!(structured(&result)["tabCount"], json!(1));
        // Not refined.
        let saved = load_browser_config().await.unwrap();
        assert_eq!(saved.connection.browser_kind(), "chromium");
    }

    // ── list_tabs / misc arms ─────────────────────────────────────────

    #[tokio::test]
    async fn list_tabs_unexpected_variant_yields_empty() {
        let (_g, _e, _d) = isolated_home().await;
        // tabs(new) succeeds, but the follow-up list returns a non-List
        // variant → empty tabs in the response.
        let mut ctx = ctx_with(
            MockSession {
                tabs_new: Some(Ok(InternalTabsResult::New {
                    page: page_info("p9"),
                    index: 0,
                })),
                tabs_list: Some(Ok(InternalTabsResult::Close {
                    url: "u".to_string(),
                    index: 0,
                })),
                ..MockSession::default()
            },
            BrowserConfig::default(),
            args(&[("action", json!("new"))]),
        );
        let result = browser_tabs(&mut ctx).await.unwrap();
        assert_eq!(structured(&result)["tabCount"], json!(0));
    }

    #[tokio::test]
    async fn mock_session_kind_protocol_and_screenshot_error() {
        let (_g, _e, dir) = isolated_home().await;
        let mut session = MockSession::default();
        assert_eq!(session.kind(), "mock");
        assert_eq!(session.protocol(), "cdp");
        session.screenshot_result = Some(Err("snap boom".to_string()));
        let mut ctx = ctx_with(session, BrowserConfig::default(), Map::new());
        let err = browser_screenshot(&mut ctx).await.unwrap_err();
        assert_eq!(err, "snap boom");
        // Screenshot failure leaves no file behind in the temp dir.
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn config_endpoint_or_default_arms() {
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: String::new(),
            },
            ..Default::default()
        };
        assert_eq!(config_endpoint_or_default(&config), DEFAULT_ENDPOINT);
        let config = BrowserConfig {
            version: 2,
            connection: BrowserConnectionConfig::Cdp {
                browser_kind: "chromium".to_string(),
                endpoint: "http://x/".to_string(),
            },
            ..Default::default()
        };
        assert_eq!(config_endpoint_or_default(&config), "http://x/");
    }
}
