//! Remote control runtime (embedded bridge) — connection lifecycle and event
//! mirroring. Command routing lives in [`commands`]; the prompt persist/finalize
//! contract lives in `agent_bridge::headless` (shared with any future headless
//! caller, so it can't drift from the frontend semantics).
//!
//! Design: see repo-root `docs/remote-control-*.md`. Currently implemented:
//!  - Step A: connect NATS, hold client, report status.
//!  - Step B: `publish_event` — mirror events to mobile at the `agent_bridge::stream` consumption point.
//!  - Step C (`commands.rs`): subscribe to `p.{pairId}.cmd.>`, route mobile commands into the GUI's persistence path.

mod commands;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Mutex;

/// Port for the embedded web client HTTP server.
const WEB_PORT: u16 = 8022;

/// Active remote connection. Holds async-nats client + JetStream context + command subscription task;
/// on stop, aborts the task and drops the client.
struct RemoteState {
    /// Raw client, kept to derive real connection state for [`status`].
    client: async_nats::Client,
    /// JetStream context: events are published through it (with `Nats-Msg-Id` idempotent dedup + written to EVT_* stream for reconnect replay).
    /// When no stream exists, publish still delivers messages to the subject; real-time subscribers still receive them (only persistence is lost), so graceful degradation.
    /// Internally holds a clone of the NATS client to keep the connection alive; dropped with RemoteState on stop.
    js: async_nats::jetstream::Context,
    nats_url: String,
    pair_id: String,
    cmd_task: tokio::task::JoinHandle<()>,
    heartbeat_task: tokio::task::JoinHandle<()>,
    web_task: tokio::task::JoinHandle<()>,
}

static STATE: Mutex<Option<RemoteState>> = Mutex::new(None);

/// Serializes concurrent `start()` calls: `STATE` can't be held across the
/// connect `await`, so without this two racing starts both pass `stop()`, both
/// spawn a command loop, and the loser's task is never aborted — its NATS
/// queue-group membership then silently steals a share of incoming commands.
static START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {
    /// The GUI backend connects to the NATS **client port** (`nats://host:4222`), NOT the browser WebSocket port.
    pub nats_url: String,
    pub pair_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub running: bool,
    pub connected: bool,
    pub nats_url: String,
    pub pair_id: String,
    pub web_url: Option<String>,
    pub error: Option<String>,
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        running: false,
        connected: false,
        nats_url: String::new(),
        pair_id: String::new(),
        web_url: None,
        error: None,
    }
}

pub async fn start(input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    // SECURITY: the command surface has no authentication (see the note above
    // `commands::handle_command`), which is only acceptable while the feature
    // is dev-gated. Enforce that premise here in the backend — hiding the nav
    // entry in the frontend is cosmetics, not a gate.
    if crate::build_info::is_release() {
        return Err(crate::AppError::Message(
            "Remote control is not available in release builds.".to_string(),
        ));
    }

    let _start_guard = START_LOCK.lock().await;

    // Stop any previous connection first (idempotent: aborts the old subscription task).
    let _ = stop();

    let client = async_nats::connect(&input.nats_url)
        .await
        .map_err(|e| crate::AppError::Message(format!("Failed to connect to NATS: {e}")))?;
    let js = async_nats::jetstream::new(client.clone());

    // Start the command subscription task (Step C).
    let cmd_task = tokio::spawn(commands::command_loop(
        client.clone(),
        input.pair_id.clone(),
    ));

    // Presence heartbeat: write KV `pairs` so clients can see desktop online state.
    let heartbeat_task = spawn_presence_heartbeat(js.clone(), input.pair_id.clone());

    // Web client HTTP server: serve remote/web/ on localhost:8022.
    let web_task = spawn_web_server();

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: input.nats_url.clone(),
        pair_id: input.pair_id.clone(),
        web_url: Some(format!("http://localhost:{WEB_PORT}")),
        error: None,
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        client,
        js,
        nats_url: input.nats_url,
        pair_id: input.pair_id,
        cmd_task,
        heartbeat_task,
        web_task,
    });
    Ok(status)
}

pub fn stop() -> RemoteStatus {
    if let Some(state) = STATE.lock().unwrap().take() {
        state.cmd_task.abort();
        state.heartbeat_task.abort();
        state.web_task.abort();
    }
    empty()
}

pub fn status() -> RemoteStatus {
    match STATE.lock().unwrap().as_ref() {
        Some(s) => {
            // Derive real health instead of reporting `connected: true` for as
            // long as STATE is occupied: the NATS client reconnects with state
            // transitions, and the command loop can die independently (failed
            // subscribe / stream end) — a dead loop processes nothing and must
            // not present as a healthy bridge.
            let loop_dead = s.cmd_task.is_finished();
            let connected = !loop_dead
                && s.client.connection_state() == async_nats::connection::State::Connected;
            RemoteStatus {
                running: true,
                connected,
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                web_url: Some(format!("http://localhost:{WEB_PORT}")),
                error: loop_dead.then(|| {
                    "Command subscription stopped; restart the remote bridge.".to_string()
                }),
            }
        }
        None => empty(),
    }
}

/// Event tap (Step B / P1): if remote is running, mirror an agent event to
/// `p.{pairId}.evt.{session}`. Returns immediately when not connected — does not block GUI event consumption.
///
/// Uses JetStream publish with `Nats-Msg-Id = {session}:{runId}:{idx}`:
///  - Idempotent: re-sent/replayed events deduplicated by broker via dupe-window;
///  - Durable: written to EVT_* stream, clients can replay on reconnect (see web `backfillActiveRun`);
///  - Graceful degradation: even without a stream, messages still reach the subject; real-time core subscribers still receive them (only persistence is lost).
///    We don't await the ack (to avoid per-token blocking) — the message is already sent on publish.
pub async fn publish_event(session_id: &str, event_type: &str, data: &str, run_id: &str, idx: i64) {
    let target = {
        let guard = STATE.lock().unwrap();
        guard.as_ref().map(|s| (s.js.clone(), s.pair_id.clone()))
    };
    let Some((js, pair_id)) = target else {
        return;
    };
    let subject = format!("p.{pair_id}.evt.{session_id}");
    let body = json!({ "type": event_type, "data": data, "runId": run_id, "idx": idx });
    let Ok(payload) = serde_json::to_vec(&body) else {
        return;
    };
    if run_id.is_empty() {
        // Events without a run_id (theoretically only early/edge cases) skip dedup, publish directly.
        let _ = js.publish(subject, payload.into()).await;
    } else {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(
            "Nats-Msg-Id",
            format!("{session_id}:{run_id}:{idx}").as_str(),
        );
        let _ = js
            .publish_with_headers(subject, headers, payload.into())
            .await;
    }
}

/// Spawn a periodic presence heartbeat that writes session state to the KV
/// bucket `pairs` under key `{pairId}`. Clients read/watch this to know the
/// desktop is online and which sessions are streaming.
///
/// The bucket is created if missing (L0 dev: no separate provisioning step).
/// The heartbeat task aborts when `stop()` is called.
fn spawn_presence_heartbeat(
    js: async_nats::jetstream::Context,
    pair_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Best-effort bucket creation; if it fails (e.g. JetStream not enabled
        // or bucket already managed externally), heartbeat writes are silently
        // skipped — presence is non-critical.
        let kv = match js
            .create_or_update_key_value(async_nats::jetstream::kv::Config {
                bucket: "pairs".to_string(),
                max_age: std::time::Duration::from_secs(120),
                ..Default::default()
            })
            .await
        {
            Ok(kv) => kv,
            Err(e) => {
                eprintln!("remote: presence KV bucket creation failed: {e}");
                return;
            }
        };

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(20));
        loop {
            interval.tick().await;
            let payload = build_presence_payload(&pair_id);
            let Ok(bytes) = serde_json::to_vec(&payload) else {
                continue;
            };
            if let Err(e) = kv.put(pair_id.as_str(), bytes.into()).await {
                eprintln!("remote: presence heartbeat write failed: {e}");
            }
        }
    })
}

/// Build the presence JSON for the current state.
fn build_presence_payload(pair_id: &str) -> serde_json::Value {
    let active_sessions: Vec<String> = crate::store::active_run_sessions().unwrap_or_default();
    let threads = crate::store::list_threads().unwrap_or_default();
    let sessions: Vec<serde_json::Value> = threads
        .iter()
        .map(|t| {
            let sid = t.agent_session_id.as_deref().unwrap_or(&t.id);
            json!({
                "id": sid,
                "name": t.title,
                "streaming": active_sessions.contains(&sid.to_string()),
            })
        })
        .collect();
    json!({
        "online": true,
        "pairId": pair_id,
        "lastHeartbeatTs": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default(),
        "sessions": sessions,
    })
}

/// Spawn a minimal HTTP server on `127.0.0.1:8022` that serves the web client
/// from `remote/web/` on disk. Reads the file on every request so edits are
/// picked up on browser refresh without rebuilding. Aborts on `stop()`.
fn spawn_web_server() -> tokio::task::JoinHandle<()> {
    // CARGO_MANIFEST_DIR = gui/src-tauri/ at compile time → repo root is two levels up.
    let web_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../remote/web")
        .canonicalize()
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../remote/web")
        });

    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", WEB_PORT)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("remote: web server bind failed on port {WEB_PORT}: {e}");
                return;
            }
        };
        eprintln!(
            "remote: web client at http://localhost:{WEB_PORT} (serving {})",
            web_dir.display()
        );

        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };
            let web_dir = web_dir.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match stream.read(&mut buf).await {
                    Ok(n) => n,
                    Err(_) => return,
                };
                let request = String::from_utf8_lossy(&buf[..n]);
                // Parse path from "GET /path HTTP/1.1" — default to index.html.
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/');
                let path = if path.is_empty() { "index.html" } else { path };
                // Prevent directory traversal.
                if path.contains("..") {
                    let resp =
                        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }
                let file_path = web_dir.join(path);
                match tokio::fs::read(&file_path).await {
                    Ok(content) => {
                        let content_type = if path.ends_with(".html") {
                            "text/html; charset=utf-8"
                        } else if path.ends_with(".js") {
                            "application/javascript"
                        } else if path.ends_with(".css") {
                            "text/css"
                        } else {
                            "application/octet-stream"
                        };
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            content.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&content).await;
                    }
                    Err(_) => {
                        let body = "Not Found";
                        let resp = format!(
                            "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                    }
                }
            });
        }
    })
}
