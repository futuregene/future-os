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

/// Bound on the event publish queue; on overflow the newest event is dropped
/// (logged) rather than blocking the agent event loop. The client recovers the
/// gap via `get_events_since` backfill on its next reattach.
const EVENT_QUEUE_CAPACITY: usize = 4096;

/// Cap on a single event's serialized size. A huge event (e.g. a large tool
/// result) would otherwise exceed the NATS 1MB user-JWT payload limit and be
/// rejected by the broker — silently leaving a permanent gap in the client's
/// event stream. Over-limit events keep their type/runId/idx (so ordering and
/// dedup still work) but ship a truncated `data` marker instead.
const MAX_EVENT_BYTES: usize = 900 * 1024;

/// One agent event queued for publishing, in agent-emission order.
struct EventPublish {
    subject: String,
    /// `Nats-Msg-Id` for JetStream dupe-window dedup; `None` for events
    /// without a run id (they can't be deduplicated anyway).
    msg_id: Option<String>,
    payload: Vec<u8>,
}

/// Active remote connection. Holds async-nats client + command/event tasks;
/// on stop, aborts the tasks and drops the client.
struct RemoteState {
    /// Raw client, kept to derive real connection state for [`status`].
    client: async_nats::Client,
    nats_url: String,
    pair_id: String,
    /// Ordered event queue → single drain task per connection. The drain holds
    /// a clone of the client (via the JetStream context) so the connection
    /// stays alive while events are in flight.
    event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    event_task: tokio::task::JoinHandle<()>,
    cmd_task: tokio::task::JoinHandle<()>,
    heartbeat_task: tokio::task::JoinHandle<()>,
    refresh_task: tokio::task::JoinHandle<()>,
    /// `None` when the web server failed to bind (port busy) — the bridge still
    /// runs, but there's no web client to point at.
    web_task: Option<tokio::task::JoinHandle<()>>,
    /// Web client URL for THIS machine (localhost); `None` when bind failed.
    web_url: Option<String>,
    /// Web client URL a phone on the same LAN can reach; `None` when bind
    /// failed or no LAN route was found.
    web_lan_url: Option<String>,
    /// Why the web server failed to bind (port busy, etc.); surfaced via
    /// [`status()`] so a running bridge with no web client explains itself.
    web_error: Option<String>,
    /// The one-shot pairing code issued at start, kept (with its expiry) so the
    /// UI can re-show it after navigation until it expires — no longer a
    /// fire-once value lost the moment you switch views.
    pairing_code: Option<String>,
    pairing_code_expires_at: Option<i64>,
}

static STATE: Mutex<Option<RemoteState>> = Mutex::new(None);

/// Why the bridge last stopped on its own (e.g. the pairing was revoked by
/// the web client). Surfaced through [`status()`] so the GUI can explain a
/// bridge that is no longer running instead of showing a bare "not running".
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

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
    /// Unix-seconds expiry of `pairing_code` (for the UI countdown); `None`
    /// when there's no code.
    pub pairing_code_expires_at: Option<i64>,
    /// Web client URL for this machine (localhost); `None` if the web server
    /// failed to bind.
    pub web_url: Option<String>,
    /// Web client URL a phone on the same LAN can reach; `None` if unavailable.
    pub web_lan_url: Option<String>,
    pub error: Option<String>,
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        running: false,
        connected: false,
        nats_url: String::new(),
        pair_id: String::new(),
        pairing_code: None,
        pairing_code_expires_at: None,
        web_url: None,
        web_lan_url: None,
        error: None,
    }
}

pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    let _start_guard = START_LOCK.lock().await;
    let _ = stop();
    *LAST_ERROR.lock().unwrap() = None;

    let (creds, pairing_code, pairing_code_expires_at) = match pairing::load_creds() {
        Some(creds) => match pairing::refresh_bridge_jwt(creds).await {
            Ok(creds) => (creds, None, None),
            Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                // A web client can revoke this pairing. Forget its unusable
                // local credential and immediately issue a replacement code
                // instead of leaving the GUI permanently stuck on startup.
                eprintln!("remote: persisted pairing was revoked; creating a new pairing");
                pairing::clear_creds()?;
                let (creds, code, exp) = pairing::create_pairing().await?;
                (creds, Some(code), exp)
            }
            Err(error) => return Err(error),
        },
        None => {
            let (creds, code, exp) = pairing::create_pairing().await?;
            (creds, Some(code), exp)
        }
    };
    let client = connect_nats(&creds).await?;
    pairing::save_creds(&creds)?;
    let js = async_nats::jetstream::new(client.clone());
    let pair_id = creds.pair_id.clone();

    // Command-id dedup cache lives OUTSIDE the command loop: credential
    // refresh swaps the loop every JWT TTL, and a cache tied to the loop would
    // be wiped each swap — retrying clients would re-execute commands (a
    // retried prompt = a duplicated user message + run).
    let reply_slots = commands::new_reply_slots();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);

    let cmd_task = tokio::spawn(commands::command_loop(
        client.clone(),
        pair_id.clone(),
        reply_slots.clone(),
    ));
    let event_task = spawn_event_publisher(js, event_rx);
    let heartbeat_task = spawn_presence_heartbeat(client.clone(), pair_id.clone());
    let refresh_task = spawn_credential_refresh(pair_id.clone(), reply_slots);
    // Bind the web server up front so a busy port is reported, not silent. A
    // failed bind is non-fatal: the bridge still runs, it just has no web UI.
    let (web_task, web_url, web_lan_url, web_error) = match bind_web_listener().await {
        Ok(listener) => (
            Some(spawn_web_server(listener)),
            Some(format!("http://localhost:{WEB_PORT}")),
            lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}")),
            None,
        ),
        Err(error) => {
            eprintln!("remote: {error}");
            (None, None, None, Some(error.to_string()))
        }
    };

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: creds.nats_url.clone(),
        pair_id: pair_id.clone(),
        pairing_code: pairing_code.clone(),
        pairing_code_expires_at,
        web_url: web_url.clone(),
        web_lan_url: web_lan_url.clone(),
        error: web_error.clone(),
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        client,
        nats_url: creds.nats_url,
        pair_id,
        event_tx,
        event_task,
        cmd_task,
        heartbeat_task,
        refresh_task,
        web_task,
        web_url,
        web_lan_url,
        web_error,
        pairing_code,
        pairing_code_expires_at,
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
        state.event_task.abort();
        state.cmd_task.abort();
        state.heartbeat_task.abort();
        state.refresh_task.abort();
        if let Some(web_task) = state.web_task {
            web_task.abort();
        }
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
            // Re-expose the pairing code until it expires so the UI keeps it
            // after navigating away and back (it's no longer a show-once value).
            let code_fresh = s.pairing_code.is_some()
                && s.pairing_code_expires_at
                    .is_some_and(|exp| exp > unix_timestamp() as i64);
            let (pairing_code, pairing_code_expires_at) = if code_fresh {
                (s.pairing_code.clone(), s.pairing_code_expires_at)
            } else {
                (None, None)
            };
            RemoteStatus {
                running: true,
                connected,
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                pairing_code,
                pairing_code_expires_at,
                web_url: s.web_url.clone(),
                web_lan_url: s.web_lan_url.clone(),
                error: loop_dead
                    .then(|| {
                        "Command subscription stopped; restart the remote bridge.".to_string()
                    })
                    .or_else(|| s.web_error.clone()),
            }
        }
        // A bridge that stopped on its own (revoked pairing) explains itself
        // through the last recorded error instead of a bare "not running".
        None => RemoteStatus {
            error: LAST_ERROR.lock().unwrap().clone(),
            ..empty()
        },
    }
}

/// If remote is running, queue an agent event for mirroring to
/// `p.{pairId}.evt.{session}`. Returns immediately when not connected — never
/// blocks GUI event consumption.
///
/// Events go through a bounded FIFO queue drained by a single task per
/// connection, so publish order matches agent emission order (the previous
/// per-event `tokio::spawn` could interleave two publishes and deliver idx
/// N+1 before idx N under load — the client dedups by (runId,idx) but renders
/// in arrival order, so reordering garbled streamed text).
///
/// The drain publishes through JetStream with `Nats-Msg-Id = {session}:{runId}:{idx}`:
///  - Idempotent: re-sent/replayed events deduplicated by broker via dupe-window;
///  - Durable: when an `EVT_{pairId}` stream exists, written to it for replay;
///  - Graceful degradation: without a matching stream the publish fails fast
///    (`no responders`) and is logged; real-time delivery to core subscribers
///    is unaffected for the events that do get through.
///
/// The ack future is deliberately NOT awaited (fire-and-forget), so a slow or
/// missing stream cannot stall the agent event loop. On queue overflow the
/// newest event is dropped and logged; the web client heals the gap via
/// `get_events_since` backfill on its next reattach.
pub fn publish_event(session_id: &str, event_type: &str, data: &str, run_id: &str, idx: i64) {
    let target = {
        let guard = STATE.lock().unwrap();
        guard
            .as_ref()
            .map(|s| (s.event_tx.clone(), s.pair_id.clone()))
    };
    let Some((tx, pair_id)) = target else {
        return;
    };
    // Guard the NATS payload cap: an oversized event is published with a
    // truncated `data` marker (type/runId/idx preserved) rather than dropped,
    // so the client's dedup cursor doesn't get a permanent hole.
    let data = cap_event_data(data);
    let body = json!({ "type": event_type, "data": data, "runId": run_id, "idx": idx });
    let Ok(payload) = serde_json::to_vec(&body) else {
        return;
    };
    let msg_id = (!run_id.is_empty()).then(|| format!("{session_id}:{run_id}:{idx}"));
    let event = EventPublish {
        subject: format!("p.{pair_id}.evt.{session_id}"),
        msg_id,
        payload,
    };
    if tx.try_send(event).is_err() {
        eprintln!("remote: event publish queue full; dropping {event_type} for {session_id}");
    }
}

/// Return `data` unchanged when it fits the payload budget, else a well-formed
/// JSON placeholder that keeps the event renderable and tells the client where
/// the full content lives (the persisted run history via `get_messages`). The
/// placeholder has no `type`-specific fields, so it's a harmless no-op in the
/// client's renderer while still advancing the (runId,idx) dedup cursor.
fn cap_event_data(data: &str) -> std::borrow::Cow<'_, str> {
    if data.len() <= MAX_EVENT_BYTES {
        return std::borrow::Cow::Borrowed(data);
    }
    std::borrow::Cow::Owned(format!(
        r#"{{"_truncated":true,"bytes":{},"note":"event exceeded the relay payload limit and was truncated; full content is available via get_messages"}}"#,
        data.len()
    ))
}

/// Serially publishes queued events on one connection, preserving agent
/// emission order. Exits when every sender is dropped (stop, or a credential
/// refresh that swapped in a new queue): on refresh the old drain is NOT
/// aborted — it keeps its JetStream context (and thus the old client) alive
/// until its backlog is flushed, avoiding a mid-stream gap at the swap point.
fn spawn_event_publisher(
    js: async_nats::jetstream::Context,
    mut rx: tokio::sync::mpsc::Receiver<EventPublish>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let result = match event.msg_id {
                Some(msg_id) => {
                    let mut headers = async_nats::HeaderMap::new();
                    headers.insert("Nats-Msg-Id", msg_id.as_str());
                    js.publish_with_headers(event.subject, headers, event.payload.into())
                        .await
                }
                None => js.publish(event.subject, event.payload.into()).await,
            };
            // Ack future dropped on purpose (fire-and-forget, see publish_event).
            if let Err(error) = result {
                eprintln!("remote: event publish failed: {error}");
            }
        }
    })
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

fn spawn_credential_refresh(
    pair_id: String,
    reply_slots: commands::ReplySlots,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Some(creds) = pairing::load_creds().filter(|creds| creds.pair_id == pair_id) else {
                return;
            };
            tokio::time::sleep(pairing::refresh_delay(&creds)).await;
            let refreshed = match pairing::refresh_bridge_jwt(creds).await {
                Ok(creds) => creds,
                Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                    // The pairing was revoked (web-side unpair, or this desktop
                    // re-paired elsewhere). Retrying forever would keep a
                    // zombie bridge that can never work again while the GUI
                    // shows "running": drop the dead credential, record why,
                    // and stop the bridge. `stop()` aborts this very task, but
                    // abort only lands at the next await and we return here.
                    eprintln!("remote: pairing was revoked on the server; stopping bridge");
                    *LAST_ERROR.lock().unwrap() = Some(
                        "Pairing was revoked (web unpair or re-pair). Start again to create a new pairing."
                            .to_string(),
                    );
                    let _ = pairing::clear_creds();
                    let _ = stop();
                    return;
                }
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
            let js = async_nats::jetstream::new(client.clone());
            let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
            let new_event = spawn_event_publisher(js, event_rx);
            let new_cmd = tokio::spawn(commands::command_loop(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
            ));
            let new_heartbeat = spawn_presence_heartbeat(client.clone(), pair_id.clone());
            // Hold the STATE lock across the generation check AND the creds
            // save: saving outside the lock raced `unpair()` (stop → clear
            // creds) and could resurrect a just-revoked credential file.
            let mut guard = STATE.lock().unwrap();
            let Some(state) = guard.as_mut().filter(|state| state.pair_id == pair_id) else {
                new_event.abort();
                new_cmd.abort();
                new_heartbeat.abort();
                return;
            };
            if let Err(error) = pairing::save_creds(&refreshed) {
                eprintln!("remote: save refreshed credential failed: {error}");
            }
            let old_cmd = std::mem::replace(&mut state.cmd_task, new_cmd);
            let old_heartbeat = std::mem::replace(&mut state.heartbeat_task, new_heartbeat);
            let old_event = std::mem::replace(&mut state.event_task, new_event);
            state.event_tx = event_tx;
            state.client = client;
            state.nats_url = refreshed.nats_url;
            old_cmd.abort();
            old_heartbeat.abort();
            // The old event drain is deliberately NOT aborted: dropping the
            // handle detaches it, and it exits on its own after flushing its
            // backlog — no event gap at the swap point.
            drop(old_event);
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

/// Cap on concurrent accepted web-client connections. Acquired BEFORE `accept`
/// so a flood of idle sockets can't exhaust file descriptors (the accept loop
/// blocks at capacity instead of parking unbounded tasks).
const WEB_MAX_CONNECTIONS: usize = 32;

/// A client that connects and never sends a request can't hold a task + fd
/// open indefinitely; its read times out and the connection is dropped.
const WEB_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// `remote/web/` on disk — two levels up from CARGO_MANIFEST_DIR (gui/src-tauri/).
fn web_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../remote/web")
        .canonicalize()
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../remote/web")
        })
}

/// Bind the web-client listener up front (in `start()`) so a busy port surfaces
/// in the returned status instead of a silent web_url that goes nowhere.
async fn bind_web_listener() -> Result<tokio::net::TcpListener, crate::AppError> {
    tokio::net::TcpListener::bind(("0.0.0.0", WEB_PORT))
        .await
        .map_err(|error| {
            crate::AppError::Message(format!(
                "web server bind on port {WEB_PORT} failed: {error}"
            ))
        })
}

/// Best-effort LAN IPv4 address so a phone on the same network can reach the
/// `0.0.0.0` web client (the GUI only knows `localhost`). Uses the classic
/// "connect a UDP socket and read the local endpoint" trick, which selects a
/// default route without sending any packets; `None` when there's no route.
fn lan_ip() -> Option<String> {
    use std::net::{IpAddr, UdpSocket};
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4.to_string()),
        _ => None,
    }
}

/// Serve the web client from `remote/web/` on the already-bound listener.
/// Reads each file per request so edits are picked up on browser refresh
/// without rebuilding. Aborts on `stop()`.
fn spawn_web_server(listener: tokio::net::TcpListener) -> tokio::task::JoinHandle<()> {
    let web_dir = web_dir();
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(WEB_MAX_CONNECTIONS));
    tokio::spawn(async move {
        eprintln!(
            "remote: web client at http://localhost:{WEB_PORT} (serving {})",
            web_dir.display()
        );
        loop {
            // Acquire the permit BEFORE accepting: at capacity the loop blocks
            // here instead of accepting sockets it can't serve.
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break, // semaphore closed
            };
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => {
                    drop(permit);
                    continue;
                }
            };
            let web_dir = web_dir.clone();
            tokio::spawn(async move {
                let _permit = permit; // held until the handler returns
                handle_web_request(&mut stream, &web_dir).await;
            });
        }
    })
}

async fn handle_web_request(stream: &mut tokio::net::TcpStream, web_dir: &std::path::Path) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 8192];
    let n = match tokio::time::timeout(WEB_READ_TIMEOUT, stream.read(&mut buf)).await {
        Ok(Ok(n)) => n,
        _ => return, // read error or a client that never sent a request
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
        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
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
}
