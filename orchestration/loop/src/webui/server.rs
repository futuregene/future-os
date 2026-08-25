//! Minimal async HTTP/1.1 server on raw tokio TCP — loopback only, no
//! external web framework (keeps the loop crate dependency-free of axum /
//! hyper for a local dashboard). Handles GET/POST, JSON responses, path
//! prefixes, and long-lived SSE streams.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::api;

/// Live-state fingerprint pushed over SSE whenever it changes.
struct Snapshot {
    overview: serde_json::Value,
    goals: serde_json::Value,
}

fn snapshot(store: &crate::store::Store) -> Snapshot {
    // A read failure must never kill the stream — push empty payloads and
    // let the next tick retry (the CLI keeps working regardless).
    let overview = api::overview(store)
        .ok()
        .and_then(|o| serde_json::to_value(o).ok())
        .unwrap_or_else(|| serde_json::json!({"error": "projection failed"}));
    let goals = api::goals_push(store)
        .ok()
        .and_then(|g| serde_json::to_value(g).ok())
        .unwrap_or_else(|| serde_json::json!([]));
    Snapshot { overview, goals }
}

fn fingerprint(s: &Snapshot) -> String {
    // Cheap change detector: serialize once per tick (projections are small).
    // `content_digest` already returns a hex string — no extra formatting.
    format!(
        "{}:{}",
        crate::store::content_digest(s.overview.to_string().as_bytes()),
        crate::store::content_digest(s.goals.to_string().as_bytes())
    )
}

pub async fn run_server(root: String, port: u16, open_browser: bool) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr} (is another `future loop ui` already running?)"))?;
    let port = listener.local_addr()?.port();
    let root = Arc::new(root);
    let (tx, _) = watch::channel(fingerprint(&snapshot(&crate::store::Store::open(&root)?)));
    let generation = Arc::new(AtomicU64::new(0));

    // Broadcaster: recompute projections; push only when content changed.
    {
        let root = root.clone();
        let tx = tx.clone();
        let generation = generation.clone();
        tokio::spawn(async move {
            let mut last = tx.borrow().clone();
            loop {
                tokio::time::sleep(Duration::from_millis(1200)).await;
                generation.fetch_add(1, Ordering::Relaxed);
                let Ok(store) = crate::store::Store::open(&root) else {
                    continue;
                };
                let fp = fingerprint(&snapshot(&store));
                if fp != last {
                    last = fp.clone();
                    let _ = tx.send(fp);
                }
            }
        });
    }

    println!("future loop ui — dashboard at http://127.0.0.1:{port}/  (root {root})");
    println!("Ctrl-C to stop; state is read live from the loop ledger.");
    if open_browser {
        open_in_browser(&format!("http://127.0.0.1:{port}/"));
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let root = root.clone();
        let rx = tx.subscribe();
        tokio::spawn(async move {
            let _ = handle(stream, root, rx).await;
        });
    }
}

#[cfg(target_os = "macos")]
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}
#[cfg(target_os = "windows")]
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
}
#[cfg(all(unix, not(target_os = "macos")))]
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<Option<Request>> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    // Read headers.
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_HEADER_BYTES {
            anyhow::bail!("headers too large");
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None); // peer closed before a full request
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_length > MAX_BODY_BYTES {
        anyhow::bail!("body too large");
    }
    let mut body = buf.split_off(header_end);
    while body.len() < content_length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    Ok(Some(Request {
        method,
        path,
        query,
        body,
    }))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn json_response(status: u16, value: &serde_json::Value) -> Vec<u8> {
    let body = value.to_string();
    respond(
        status,
        "application/json; charset=utf-8",
        body.as_bytes(),
        &[],
    )
}

fn error_response(status: u16, message: &str) -> Vec<u8> {
    json_response(status, &serde_json::json!({"ok": false, "error": message}))
}

fn respond(
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\ncache-control: no-store\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("connection: close\r\n\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(percent_decode(v));
            }
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn handle(
    mut stream: TcpStream,
    root: Arc<String>,
    rx: watch::Receiver<String>,
) -> Result<()> {
    stream.set_nodelay(true)?;
    let Some(req) = read_request(&mut stream).await? else {
        return Ok(());
    };

    // SSE: long-lived, bypasses the normal response path.
    if req.method == "GET" && req.path == "/api/stream" {
        return serve_sse(stream, root, rx).await;
    }

    let response = route(&req, &root);
    stream.write_all(&response).await?;
    stream.shutdown().await?;
    Ok(())
}

fn route(req: &Request, root: &str) -> Vec<u8> {
    let store = match crate::store::Store::open(root) {
        Ok(s) => s,
        Err(e) => return error_response(500, &format!("open store: {e:#}")),
    };
    let path = req.path.trim_end_matches('/');
    if req.method == "GET" && (path.is_empty() || path == "/index.html") {
        return respond(
            200,
            "text/html; charset=utf-8",
            super::page::PAGE.as_bytes(),
            &[],
        );
    }
    if req.method == "GET" && path == "/api/overview" {
        return match api::overview(&store) {
            Ok(o) => json_response(200, &serde_json::json!({"ok": true, "data": o})),
            Err(e) => error_response(500, &format!("{e:#}")),
        };
    }
    if req.method == "GET" && path == "/api/config" {
        return json_response(
            200,
            &serde_json::json!({"ok": true, "data": api::model_config()}),
        );
    }
    // /api/goals/{id}/... segments
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() >= 2 && segments[0] == "api" && segments[1] == "goals" {
        let goal_id = percent_decode(segments[2]);
        match (req.method.as_str(), segments.get(3).copied()) {
            ("GET", None) => {
                return match api::goal_detail(&store, &goal_id) {
                    Ok(Some(d)) => json_response(200, &serde_json::json!({"ok": true, "data": d})),
                    Ok(None) => error_response(404, &format!("goal {goal_id} not found")),
                    Err(e) => error_response(500, &format!("{e:#}")),
                };
            }
            ("GET", Some("runs")) => {
                let limit = query_param(&req.query, "limit")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(200);
                return match api::runs_page(&store, &goal_id, limit) {
                    Ok(Some(r)) => json_response(200, &serde_json::json!({"ok": true, "data": r})),
                    Ok(None) => error_response(404, &format!("goal {goal_id} not found")),
                    Err(e) => error_response(500, &format!("{e:#}")),
                };
            }
            ("GET", Some("events")) => {
                let limit = query_param(&req.query, "limit")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(200);
                return match api::events_page(&store, &goal_id, limit) {
                    Ok(Some(r)) => json_response(200, &serde_json::json!({"ok": true, "data": r})),
                    Ok(None) => error_response(404, &format!("goal {goal_id} not found")),
                    Err(e) => error_response(500, &format!("{e:#}")),
                };
            }
            ("POST", Some("gate")) => {
                return post_json(&req.body, root, &goal_id, PostKind::Gate);
            }
            ("POST", Some("lifecycle")) => {
                return post_json(&req.body, root, &goal_id, PostKind::Lifecycle);
            }
            _ => return error_response(404, "unknown route"),
        }
    }
    error_response(404, "unknown route")
}

enum PostKind {
    Gate,
    Lifecycle,
}

fn post_json(body: &[u8], root: &str, goal_id: &str, kind: PostKind) -> Vec<u8> {
    // Mutations need a mutable store (append takes &mut self).
    let mut store = match crate::store::Store::open(root) {
        Ok(s) => s,
        Err(e) => return error_response(500, &format!("open store: {e:#}")),
    };
    let result: Result<String> = (|| match kind {
        PostKind::Gate => {
            let body: api::GateResolveBody = serde_json::from_slice(body)
                .context("invalid JSON body (expected {\"todo_id\",\"decision\"})")?;
            api::resolve_gate(&mut store, goal_id, &body)
        }
        PostKind::Lifecycle => {
            let body: api::LifecycleBody = serde_json::from_slice(body)
                .context("invalid JSON body (expected {\"action\":\"cancel\"})")?;
            api::set_lifecycle(&mut store, goal_id, &body)
        }
    })();
    match result {
        Ok(message) => json_response(200, &serde_json::json!({"ok": true, "message": message})),
        Err(e) => error_response(400, &format!("{e:#}")),
    }
}

async fn send_snapshot(stream: &mut TcpStream, root: &str) -> Result<()> {
    let store = crate::store::Store::open(root)?;
    let snap = snapshot(&store);
    let frame = format!(
        "event: overview\ndata: {}\n\nevent: goals\ndata: {}\n\n",
        snap.overview, snap.goals
    );
    stream.write_all(frame.as_bytes()).await?;
    Ok(())
}

async fn serve_sse(
    mut stream: TcpStream,
    root: Arc<String>,
    mut rx: watch::Receiver<String>,
) -> Result<()> {
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-store\r\nconnection: keep-alive\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    // Initial payload so the page renders without waiting for the first tick.
    if send_snapshot(&mut stream, &root).await.is_err() {
        return Ok(());
    }
    loop {
        // Push on change; heartbeat comment keeps proxies/clients alive.
        let changed = tokio::time::timeout(Duration::from_secs(15), rx.changed()).await;
        match changed {
            Ok(Ok(())) => {
                if send_snapshot(&mut stream, &root).await.is_err() {
                    return Ok(());
                }
            }
            Ok(Err(_)) => return Ok(()), // broadcaster gone
            Err(_) => {
                if stream.write_all(b": ping\n\n").await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}
