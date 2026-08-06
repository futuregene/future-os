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
    let launcher = find_browser_launcher(executable_path.as_deref());
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

async fn endpoint_for(args: &Map<String, Value>) -> String {
    let config = load_browser_config().await.unwrap_or_default();
    string_arg(args, "endpoint").unwrap_or_else(|| {
        let endpoint = config.connection.endpoint();
        if endpoint.is_empty() {
            DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint.to_string()
        }
    })
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

async fn wait_for_saved_endpoint(
    fallback_endpoint: &str,
    timeout_ms: u64,
) -> Result<String, String> {
    let config = load_browser_config().await?;
    let started_endpoint = {
        let ep = config.connection.endpoint();
        if ep.is_empty() {
            fallback_endpoint.to_string()
        } else {
            ep.to_string()
        }
    };
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
