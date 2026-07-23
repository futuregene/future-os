//! Remote control runtime (embedded bridge) — connection lifecycle and event
//! mirroring. Command routing lives in [`commands`]; the prompt persist/finalize
//! contract lives in `agent_bridge::headless` (shared with any future headless
//! caller, so it can't drift from the frontend semantics).
//!
//! Design: see `gui/DEV_MD/remote-control-*.md`. The embedded bridge connects
//! with a short-lived, pair-scoped NATS user JWT, mirrors agent events, routes
//! Web/App commands through the GUI persistence path, publishes presence, and
//! refreshes credentials before expiry.

mod commands;
pub(crate) mod pairing;

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
    refresh_task: tokio::task::JoinHandle<()>,
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
pub struct RemoteStartInput {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub running: bool,
    pub connected: bool,
    pub nats_url: String,
    pub pair_id: String,
    /// One-shot pairing code (base64url) returned only by a successful start, for the UI to display/copy.
    pub pairing_code: Option<String>,
    pub web_url: Option<String>,
    pub error: Option<String>,
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        running: false,
        connected: false,
        nats_url: String::new(),
        pair_id: String::new(),
        pairing_code: None,
        web_url: None,
        error: None,
    }
}

pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    let _start_guard = START_LOCK.lock().await;
    let _ = stop();

    let (creds, pairing_code) = match pairing::load_creds() {
        Some(creds) => (pairing::refresh_bridge_jwt(creds).await?, None),
        None => {
            let (creds, code) = pairing::create_pairing().await?;
            (creds, Some(code))
        }
    };
    let client = connect_nats(&creds).await?;
    pairing::save_creds(&creds)?;
    let js = async_nats::jetstream::new(client.clone());
    let pair_id = creds.pair_id.clone();

    let cmd_task = tokio::spawn(commands::command_loop(client.clone(), pair_id.clone()));
    let heartbeat_task = spawn_presence_heartbeat(client.clone(), pair_id.clone());
    let refresh_task = spawn_credential_refresh(pair_id.clone());
    let web_task = spawn_web_server();

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: creds.nats_url.clone(),
        pair_id: pair_id.clone(),
        pairing_code,
        web_url: Some(format!("http://localhost:{WEB_PORT}")),
        error: None,
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        client,
        js,
        nats_url: creds.nats_url,
        pair_id,
        cmd_task,
        heartbeat_task,
        refresh_task,
        web_task,
    });
    Ok(status)
}

async fn connect_nats(
    creds: &pairing::PairingCreds,
) -> Result<async_nats::Client, crate::AppError> {
    let key_pair = std::sync::Arc::new(
        nkeys::KeyPair::from_seed(&creds.nkey_seed)
            .map_err(|error| crate::AppError::Message(format!("Invalid desktop NKey: {error}")))?,
    );
    let options = async_nats::ConnectOptions::with_jwt(creds.user_jwt.clone(), move |nonce| {
        let key_pair = key_pair.clone();
        async move { key_pair.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .custom_inbox_prefix(format!("p.{}.rep.{}", creds.pair_id, creds.desktop_id));
    options
        .connect(&creds.nats_url)
        .await
        .map_err(|error| crate::AppError::Message(format!("Failed to connect to NATS: {error}")))
}

/// Drop the persisted pairing and stop the bridge (the desktop "unpair").
pub async fn unpair() -> Result<RemoteStatus, crate::AppError> {
    if let Some(creds) = pairing::load_creds() {
        pairing::revoke_pairing(&creds).await?;
    }
    let status = stop();
    pairing::clear_creds()?;
    Ok(status)
}

pub fn stop() -> RemoteStatus {
    if let Some(state) = STATE.lock().unwrap().take() {
        let pair_id = state.pair_id.clone();
        let client = state.client.clone();
        tauri::async_runtime::spawn(async move {
            let subject = format!("p.{pair_id}.presence");
            let payload = serde_json::to_vec(&json!({
                "online": false,
                "pairId": pair_id,
                "lastHeartbeatTs": unix_timestamp(),
                "sessions": [],
            }))
            .unwrap_or_default();
            let _ = client.publish(subject, payload.into()).await;
            let _ = client.flush().await;
        });
        state.cmd_task.abort();
        state.heartbeat_task.abort();
        state.refresh_task.abort();
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
                pairing_code: None,
                web_url: Some(format!("http://localhost:{WEB_PORT}")),
                error: loop_dead.then(|| {
                    "Command subscription stopped; restart the remote bridge.".to_string()
                }),
            }
        }
        None => empty(),
    }
}

/// If remote is running, mirror an agent event to
/// `p.{pairId}.evt.{session}`. Returns immediately when not connected — does not block GUI event consumption.
///
/// Uses JetStream publish with `Nats-Msg-Id = {session}:{runId}:{idx}`:
///  - Idempotent: re-sent/replayed events deduplicated by broker via dupe-window;
///  - Durable: when an `EVT_{pairId}` stream exists, written to it for replay;
///  - Graceful degradation: even without a stream, the message still reaches the
///    subject (real-time core subscribers receive it; only persistence is lost).
///
/// The publish is **fire-and-forget**: we spawn the ack-waiting future instead of
/// awaiting it, so a missing stream (no matching JetStream stream → the server
/// returns an error ack immediately) cannot block the agent event loop. This is
/// what lets the bridge degrade gracefully if stream provisioning is
/// temporarily unavailable.
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
        tokio::spawn(async move {
            let _ = js.publish(subject, payload.into()).await;
        });
    } else {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(
            "Nats-Msg-Id",
            format!("{session_id}:{run_id}:{idx}").as_str(),
        );
        tokio::spawn(async move {
            let _ = js
                .publish_with_headers(subject, headers, payload.into())
                .await;
        });
    }
}

fn spawn_presence_heartbeat(
    client: async_nats::Client,
    pair_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(20));
        loop {
            interval.tick().await;
            let payload = build_presence_payload(&pair_id);
            let Ok(bytes) = serde_json::to_vec(&payload) else {
                continue;
            };
            if let Err(e) = client
                .publish(format!("p.{pair_id}.presence"), bytes.into())
                .await
            {
                eprintln!("remote: presence heartbeat write failed: {e}");
            }
        }
    })
}

fn spawn_credential_refresh(pair_id: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Some(creds) = pairing::load_creds().filter(|creds| creds.pair_id == pair_id) else {
                return;
            };
            tokio::time::sleep(pairing::refresh_delay(&creds)).await;
            let refreshed = match pairing::refresh_bridge_jwt(creds).await {
                Ok(creds) => creds,
                Err(error) => {
                    eprintln!("remote: credential refresh failed: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    continue;
                }
            };
            let client = match connect_nats(&refreshed).await {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("remote: reconnect with refreshed credential failed: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    continue;
                }
            };
            if let Err(error) = pairing::save_creds(&refreshed) {
                eprintln!("remote: save refreshed credential failed: {error}");
            }
            let js = async_nats::jetstream::new(client.clone());
            let new_cmd = tokio::spawn(commands::command_loop(client.clone(), pair_id.clone()));
            let new_heartbeat = spawn_presence_heartbeat(client.clone(), pair_id.clone());
            let mut guard = STATE.lock().unwrap();
            let Some(state) = guard.as_mut().filter(|state| state.pair_id == pair_id) else {
                new_cmd.abort();
                new_heartbeat.abort();
                return;
            };
            let old_cmd = std::mem::replace(&mut state.cmd_task, new_cmd);
            let old_heartbeat = std::mem::replace(&mut state.heartbeat_task, new_heartbeat);
            state.client = client;
            state.js = js;
            state.nats_url = refreshed.nats_url;
            old_cmd.abort();
            old_heartbeat.abort();
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

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
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
