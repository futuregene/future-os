//! Remote control runtime (embedded bridge) — connection lifecycle and event
//! mirroring. Command routing lives in [`commands`]; the prompt persist/finalize
//! contract lives in `agent_bridge::headless` (shared with any future headless
//! caller, so it can't drift from the frontend semantics).
//!
//! Design: see `desktop/DEV_MD/remote-control-*.md`. The embedded bridge connects
//! with a short-lived, pair-scoped NATS user JWT, mirrors agent events, routes
//! Web/App commands through the GUI persistence path, publishes presence, and
//! refreshes credentials before expiry.

mod commands;
pub(crate) mod pairing;
#[cfg(test)]
pub(crate) mod test_support;
mod transfer;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Arc, Mutex,
};

/// Port for the embedded web client HTTP server.
const WEB_PORT: u16 = 8022;
/// A shutdown notification is advisory: never delay closing the desktop for a
/// slow or unreachable broker, but give a healthy connection a short window to
/// flush the packet before its tasks are torn down.
const DISCONNECT_NOTICE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(750);

/// Bound on the event publish queue; on overflow the newest event is dropped
/// (logged) rather than blocking the agent event loop. The client recovers the
/// gap via `get_events_since` backfill on its next reattach.
const EVENT_QUEUE_CAPACITY: usize = 4096;

/// Rate-limited drop reporting for the remote event mirror: one line when a
/// drop episode starts, one per 10s while it persists, one on recovery — instead
/// of the per-event line that flooded the terminal the moment the queue
/// saturated (e.g. NATS offline). A dropped event is never data loss; the
/// client heals the gap via `get_events_since` backfill.
struct DropCounters {
    /// An episode (queue full / NATS offline) is active until a successful enqueue.
    dropping: AtomicBool,
    /// Cumulative drops since the episode started; reset on recovery.
    dropped: AtomicU64,
    /// Epoch-ms of the last emitted drop line; gates the 10s periodic line.
    last_report: AtomicU64,
}

impl DropCounters {
    const fn new() -> Self {
        Self {
            dropping: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            last_report: AtomicU64::new(0),
        }
    }

    /// Returns the line to print for a dropped event, or `None` when the episode
    /// is already being reported at full cadence. Event-driven: called on the
    /// hot path every event passes through, so it keeps reporting even while
    /// the drain task is blocked on `publish().await` and can't reach its own
    /// timer. `now` is injected so tests can advance time deterministically.
    fn record_drop(
        &self,
        why: &str,
        event_type: &str,
        session_id: &str,
        now: u64,
    ) -> Option<String> {
        let first = !self.dropping.swap(true, Ordering::Relaxed);
        let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        let last = self.last_report.swap(now, Ordering::Relaxed);
        if first {
            Some(format!(
                "remote: {why}; dropping {event_type} for {session_id} (backfill on reconnect heals the gap)"
            ))
        } else if total >= 10 && now.saturating_sub(last) >= 10_000 {
            Some(format!(
                "remote: {why}; dropped {total} events so far for {session_id}"
            ))
        } else {
            None
        }
    }

    /// One-shot recovery line when a drop episode ends (the first event that
    /// enqueues successfully). Returns `None` when no episode was active, and
    /// resets the episode counters so the next episode starts fresh.
    fn report_recovery(&self) -> Option<String> {
        if self.dropping.swap(false, Ordering::Relaxed) {
            let dropped = self.dropped.swap(0, Ordering::Relaxed);
            self.last_report.store(0, Ordering::Relaxed);
            Some(format!(
                "remote: event publish recovered; dropped {dropped} events during the backlog"
            ))
        } else {
            None
        }
    }
}

static DROP_COUNTERS: DropCounters = DropCounters::new();

/// Cap on a single event's serialized size. A huge event (e.g. a large tool
/// result) would otherwise exceed the NATS 1MB user-JWT payload limit and be
/// rejected by the broker — silently leaving a permanent gap in the client's
/// event stream. Over-limit events keep their type/runId/idx (so ordering and
/// dedup still work) but ship a truncated `data` marker instead.
const MAX_EVENT_BYTES: usize = 900 * 1024;

/// One agent event queued for publishing, in agent-emission order.
struct EventPublish {
    subject: String,
    payload: Vec<u8>,
}

/// Active remote connection. Holds async-nats client + command/event tasks;
/// on stop, aborts the tasks and drops the client.
struct RemoteState {
    /// Raw client, kept to derive real connection state for [`status`].
    client: async_nats::Client,
    nats_url: String,
    pair_id: String,
    desktop_id: String,
    desktop_public_key: String,
    bridge_instance_id: String,
    /// Ordered event queue → single drain task per connection. The drain holds
    /// a clone of the client so the connection stays alive while events are in
    /// flight.
    event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    event_task: tokio::task::JoinHandle<()>,
    cmd_task: tokio::task::JoinHandle<()>,
    transfer_task: tokio::task::JoinHandle<()>,
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
    /// The one-shot pairing code issued at start, kept (with its expiry) so the
    /// UI can re-show it after navigation until it expires — no longer a
    /// fire-once value lost the moment you switch views.
    pairing_code: Option<String>,
    pairing_code_expires_at: Option<i64>,
    /// New pairings remain pending until the client and bridge complete the
    /// signed application-level handshake.
    pairing_confirmed: Arc<AtomicBool>,
}

static STATE: Mutex<Option<RemoteState>> = Mutex::new(None);

/// Why the bridge last stopped on its own (e.g. the pairing was revoked by
/// the web client), as a machine-readable category the UI localizes
/// (`error.<code>`). Surfaced through [`status()`] so the GUI can explain a
/// bridge that is no longer running instead of showing a bare "not running".
static LAST_ERROR_CODE: Mutex<Option<String>> = Mutex::new(None);

/// Serializes concurrent `start()` calls: `STATE` can't be held across the
/// connect `await`, so without this two racing starts both pass `stop()`, both
/// spawn a command loop, and the loser's task is never aborted — its NATS
/// queue-group membership then silently steals a share of incoming commands.
static START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// User/runtime intent for the bridge to stay online. A transient startup
/// failure leaves this set so the background retry worker can recover when the
/// network returns; an explicit stop clears it and prevents resurrection.
static START_REQUESTED: AtomicBool = AtomicBool::new(false);
/// At most one process-lifetime startup retry worker may run at once.
static START_RETRY_RUNNING: AtomicBool = AtomicBool::new(false);
/// A finished critical task is rebuilt automatically. Bound repeated
/// reconnects so a deterministic panic cannot spin forever.
static RUNTIME_RECONNECT_RUNNING: AtomicBool = AtomicBool::new(false);
static RUNTIME_RECONNECT_ATTEMPTS: AtomicU8 = AtomicU8::new(0);
const MAX_RUNTIME_RECONNECT_ATTEMPTS: u8 = 3;
static WEB_RECONNECT_RUNNING: AtomicBool = AtomicBool::new(false);
static WEB_RECONNECT_ATTEMPTS: AtomicU8 = AtomicU8::new(0);
const MAX_WEB_RECONNECT_ATTEMPTS: u8 = 3;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub running: bool,
    pub connected: bool,
    /// The bridge is rebuilding a failed connection generation.
    pub reconnecting: bool,
    pub nats_url: String,
    pub pair_id: String,
    /// One-shot pairing code (base64url) returned only by a successful start, for the UI to display/copy.
    pub pairing_code: Option<String>,
    /// Unix-seconds expiry of `pairing_code` (for the UI countdown); `None`
    /// when there's no code.
    pub pairing_code_expires_at: Option<i64>,
    /// Desktop identity bound into the QR invitation and signed handshake.
    pub desktop_id: String,
    pub desktop_public_key: String,
    /// Web client URL for this machine (localhost); `None` if the web server
    /// failed to bind.
    pub web_url: Option<String>,
    /// Web client URL a phone on the same LAN can reach; `None` if unavailable.
    pub web_lan_url: Option<String>,
    /// Machine-readable reason the bridge isn't healthy (e.g. `network`,
    /// `revoked`, `server`, `reconnect_required`, `web_bind`). The UI localizes this via
    /// `error.<code>`; it is the preferred signal over [`Self::error`].
    pub error_code: Option<String>,
    /// Human-readable error text, used only when [`Self::error_code`] is `None`
    /// (an uncategorized local failure). When a code is present the UI shows
    /// the localized message and ignores this field.
    pub error: Option<String>,
}

fn retryable_start_status(status: &RemoteStatus) -> bool {
    !status.running
        && matches!(
            status.error_code.as_deref(),
            Some("network") | Some("server")
        )
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        running: false,
        connected: false,
        reconnecting: false,
        nats_url: String::new(),
        pair_id: String::new(),
        pairing_code: None,
        pairing_code_expires_at: None,
        desktop_id: String::new(),
        desktop_public_key: String::new(),
        web_url: None,
        web_lan_url: None,
        error_code: None,
        error: None,
    }
}

pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    // An explicit user reconnect gets fresh automatic-reconnect budgets.
    RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    START_REQUESTED.store(true, Ordering::Release);
    let result = start_once(true).await;
    if result.as_ref().is_ok_and(retryable_start_status) {
        spawn_start_retry();
    }
    result
}

async fn start_once(replace_existing: bool) -> Result<RemoteStatus, crate::AppError> {
    let _start_guard = START_LOCK.lock().await;
    if !START_REQUESTED.load(Ordering::Acquire) {
        return Ok(empty());
    }
    if !replace_existing {
        let current = status();
        if current.running {
            return Ok(current);
        }
    }
    let _ = stop_runtime();
    *LAST_ERROR_CODE.lock().unwrap() = None;

    // A remote/server failure here (offline, revoked, HTTP error) is not a
    // program fault — surface it as a localized, not-running status instead of
    // throwing a raw transport string at the UI. Local failures (NKey, disk)
    // keep propagating as `Err`.
    let (creds, pairing_code, pairing_code_expires_at) = match establish().await {
        Ok(value) => value,
        Err(error) => {
            eprintln!("remote: start failed at establish: {error}");
            return start_failure(error);
        }
    };
    if !START_REQUESTED.load(Ordering::Acquire) {
        return Ok(empty());
    }
    let client = match connect_nats(&creds).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("remote: start failed at connect_nats: {error}");
            return start_failure(error);
        }
    };
    if !START_REQUESTED.load(Ordering::Acquire) {
        return Ok(empty());
    }
    let pairing_confirmed = Arc::new(AtomicBool::new(pairing_code.is_none()));
    if pairing_confirmed.load(Ordering::Acquire) {
        pairing::save_creds(&creds)?;
    }
    let desktop_public_key = pairing::public_key(&creds)?;
    let bridge_instance_id = format!("bridge_{}", nkeys::KeyPair::new_user().public_key());
    let pair_id = creds.pair_id.clone();

    // Command-id dedup cache lives OUTSIDE the command loop: credential
    // refresh swaps the loop every JWT TTL, and a cache tied to the loop would
    // be wiped each swap — retrying clients would re-execute commands (a
    // retried prompt = a duplicated user message + run).
    let reply_slots = commands::new_reply_slots();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
    let handshake_state = commands::HandshakeState::new(
        creds.clone(),
        pairing_confirmed.clone(),
        bridge_instance_id.clone(),
    );

    let cmd_task = tokio::spawn(commands::command_loop(
        client.clone(),
        pair_id.clone(),
        reply_slots.clone(),
        handshake_state.clone(),
    ));
    let transfer_task = transfer::spawn_transfer_loop(
        client.clone(),
        pair_id.clone(),
        handshake_state.active_flag(),
    );
    let event_task = spawn_event_publisher(client.clone(), event_rx);
    let heartbeat_task =
        spawn_presence_heartbeat(client.clone(), pair_id.clone(), bridge_instance_id.clone());
    let refresh_task = spawn_credential_refresh(
        pair_id.clone(),
        reply_slots,
        pairing_confirmed.clone(),
        handshake_state,
    );
    // Bind the web server up front so a busy port is reported, not silent. A
    // failed bind is non-fatal: the bridge still runs, it just has no web UI.
    let (web_task, web_url, web_lan_url) = match bind_web_listener().await {
        Ok(listener) => (
            Some(spawn_web_server(listener)),
            Some(format!("http://localhost:{WEB_PORT}")),
            lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}")),
        ),
        Err(error) => {
            eprintln!("remote: web client bind failed: {error}");
            (None, None, None)
        }
    };
    // A failed web bind is non-fatal (the bridge still runs) and is retried
    // silently before the UI is asked to intervene.
    let web_bind_failed = web_task.is_none();

    let status = RemoteStatus {
        running: true,
        connected: true,
        reconnecting: false,
        nats_url: creds.nats_url.clone(),
        pair_id: pair_id.clone(),
        pairing_code: pairing_code.clone(),
        pairing_code_expires_at,
        desktop_id: creds.desktop_id.clone(),
        desktop_public_key: desktop_public_key.clone(),
        web_url: web_url.clone(),
        web_lan_url: web_lan_url.clone(),
        error_code: None,
        error: None,
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        client,
        nats_url: creds.nats_url,
        pair_id,
        desktop_id: creds.desktop_id,
        desktop_public_key,
        bridge_instance_id,
        event_tx,
        event_task,
        cmd_task,
        transfer_task,
        heartbeat_task,
        refresh_task,
        web_task,
        web_url,
        web_lan_url,
        pairing_code,
        pairing_code_expires_at,
        pairing_confirmed,
    });
    if web_bind_failed {
        spawn_web_reconnect(status.pair_id.clone());
    }
    Ok(status)
}

/// Keep retrying a categorized startup failure on Tauri's process-lifetime
/// runtime. This covers launching while offline and a network transition during
/// the first connect. Runtime disconnects after a successful start continue to
/// use async-nats plus the credential-refresh health swap below.
fn spawn_start_retry() {
    #[cfg(test)]
    return;

    #[cfg(not(test))]
    {
        if START_RETRY_RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        tauri::async_runtime::spawn(async {
            let mut delay = std::time::Duration::from_secs(1);
            loop {
                tokio::time::sleep(delay).await;
                if !START_REQUESTED.load(Ordering::Acquire) || status().running {
                    break;
                }
                match start_once(false).await {
                    Ok(status) if status.running => break,
                    Ok(status) if retryable_start_status(&status) => {
                        delay = delay
                            .saturating_mul(2)
                            .min(std::time::Duration::from_secs(30));
                    }
                    Ok(_) | Err(_) => break,
                }
            }
            START_RETRY_RUNNING.store(false, Ordering::Release);
            // Close the tiny stop→start race: a new start may have requested
            // recovery after this worker decided to exit but before it released
            // the singleton flag. Re-arm from the latest status in that case.
            let latest = status();
            if START_REQUESTED.load(Ordering::Acquire) && retryable_start_status(&latest) {
                spawn_start_retry();
            }
        });
    }
}

/// Rebuild the whole bridge generation when a subscription task itself dies.
/// Ordinary NATS disconnects are handled inside the task; reaching this path
/// means the task exited or panicked and cannot resubscribe on its own.
fn spawn_runtime_reconnect() {
    #[cfg(test)]
    return;

    #[cfg(not(test))]
    {
        if RUNTIME_RECONNECT_RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        let attempt = RUNTIME_RECONNECT_ATTEMPTS.fetch_add(1, Ordering::AcqRel) + 1;
        eprintln!(
            "remote: critical task stopped; reconnecting bridge automatically ({attempt}/{MAX_RUNTIME_RECONNECT_ATTEMPTS})"
        );
        tauri::async_runtime::spawn(async move {
            let result = start_once(true).await;
            match &result {
                Ok(status) if retryable_start_status(status) => spawn_start_retry(),
                Err(error) => {
                    eprintln!("remote: automatic bridge reconnect failed: {error}");
                    *LAST_ERROR_CODE.lock().unwrap() = Some("reconnect_required".to_string());
                }
                _ => {}
            }
            RUNTIME_RECONNECT_RUNNING.store(false, Ordering::Release);
        });
    }
}

/// Retry only the optional local Web listener. A busy port must not tear down
/// the healthy phone/NATS bridge; after a bounded retry budget the UI offers a
/// full reconnect button, which retries the listener with a fresh generation.
fn spawn_web_reconnect(pair_id: String) {
    #[cfg(test)]
    {
        let _ = pair_id;
    }

    #[cfg(not(test))]
    {
        if WEB_RECONNECT_RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        let attempt = WEB_RECONNECT_ATTEMPTS.fetch_add(1, Ordering::AcqRel) + 1;
        eprintln!(
            "remote: local web listener unavailable; retrying ({attempt}/{MAX_WEB_RECONNECT_ATTEMPTS})"
        );
        tauri::async_runtime::spawn(async move {
            match bind_web_listener().await {
                Ok(listener) => {
                    let web_url = Some(format!("http://localhost:{WEB_PORT}"));
                    let web_lan_url = lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}"));
                    let web_task = spawn_web_server(listener);
                    let mut guard = STATE.lock().unwrap();
                    if let Some(state) = guard.as_mut().filter(|state| state.pair_id == pair_id) {
                        if let Some(previous) = state.web_task.replace(web_task) {
                            previous.abort();
                        }
                        state.web_url = web_url;
                        state.web_lan_url = web_lan_url;
                        WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
                        eprintln!("remote: local web listener reconnected");
                    } else {
                        web_task.abort();
                    }
                }
                Err(error) => eprintln!("remote: local web listener reconnect failed: {error}"),
            }
            WEB_RECONNECT_RUNNING.store(false, Ordering::Release);
        });
    }
}

/// Resolve a usable credential: refresh the persisted pairing, or — if it was
/// revoked server-side — drop it and mint a fresh pairing code. Pure control
/// plane; the NATS connect happens in [`start`] so its failure can be reported
/// the same way.
async fn establish() -> Result<(pairing::PairingCreds, Option<String>, Option<i64>), crate::AppError>
{
    match pairing::load_creds() {
        Some(creds) if creds.handshake_version != 1 => {
            // Credentials created before the signed mutual handshake have no
            // QR-bound peer identity. They cannot be upgraded safely in place.
            eprintln!("remote: replacing legacy pairing without a signed handshake");
            pairing::clear_creds()?;
            let (creds, code, exp) = pairing::create_pairing().await?;
            Ok((creds, Some(code), exp))
        }
        Some(creds) => match pairing::refresh_bridge_jwt(creds).await {
            Ok(creds) => Ok((creds, None, None)),
            Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                // A web client can revoke this pairing. Forget its unusable
                // local credential and immediately issue a replacement code
                // instead of leaving the GUI permanently stuck on startup.
                eprintln!("remote: persisted pairing was revoked; creating a new pairing");
                pairing::clear_creds()?;
                let (creds, code, exp) = pairing::create_pairing().await?;
                Ok((creds, Some(code), exp))
            }
            Err(error) => Err(error),
        },
        None => {
            let (creds, code, exp) = pairing::create_pairing().await?;
            Ok((creds, Some(code), exp))
        }
    }
}

/// Turn a start-time failure into either a localized not-running status (when
/// the cause is a categorized remote/network/server error) or an `Err` for an
/// uncategorized local fault. Records the code so a later [`status`] poll stays
/// consistent with the value returned here.
fn start_failure(error: crate::AppError) -> Result<RemoteStatus, crate::AppError> {
    match pairing::error_code(&error) {
        Some(code) => {
            eprintln!("remote: start failed [{code}]: {error}");
            *LAST_ERROR_CODE.lock().unwrap() = Some(code.to_string());
            Ok(RemoteStatus {
                error_code: Some(code.to_string()),
                ..empty()
            })
        }
        None => Err(error),
    }
}

/// The desktop bridge talks NATS **directly** (`nats://…`) to the operator's
/// own server over a trusted path; the server is this deployment's own
/// infrastructure. TLS on this hop is the operator's choice, not something we
/// can hard-assert here — the remote-link TLS invariant lives on the *mobile
/// / web* side (`wss://`, enforced in the mobile client's `assertSecureNatsUrl`
/// and the web server's own URL construction). Do NOT reintroduce a `wss://`
/// assertion on this hop: the platform hands the desktop a `nats://` URL by
/// design.
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
    options.connect(&creds.nats_url).await.map_err(|error| {
        crate::AppError::RemoteTransport(format!("Failed to connect to NATS: {error}"))
    })
}

/// Drop the persisted pairing and stop the bridge (the desktop "unpair").
pub async fn unpair() -> Result<RemoteStatus, crate::AppError> {
    // Give an online phone an immediate, authenticated signal before the
    // server-side revoke invalidates the shared NATS credentials. Revocation
    // remains the authoritative fallback when this at-most-once message cannot
    // be delivered (phone offline, broker outage, older client, etc.).
    notify_mobile_unpair().await;
    if let Some(creds) = pairing::load_creds() {
        pairing::revoke_pairing(&creds).await?;
    }
    let status = stop();
    pairing::clear_creds()?;
    Ok(status)
}

async fn notify_mobile_unpair() {
    let Some((client, pair_id, bridge_instance_id)) = STATE.lock().unwrap().as_ref().map(|state| {
        (
            state.client.clone(),
            state.pair_id.clone(),
            state.bridge_instance_id.clone(),
        )
    }) else {
        return;
    };
    let payload = serde_json::to_vec(&json!({
        "online": false,
        "unpaired": true,
        "pairId": pair_id,
        "bridgeInstanceId": bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
    }))
    .unwrap_or_default();
    let _ = client
        .publish(format!("p.{pair_id}.presence"), payload.into())
        .await;
    let _ = client.flush().await;
}

pub fn stop() -> RemoteStatus {
    START_REQUESTED.store(false, Ordering::Release);
    RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    stop_runtime()
}

/// Notify an online mobile client before an intentional desktop disconnect.
/// The mobile client treats this as immediate offline state; heartbeat expiry
/// remains the fallback for crashes, forced power-off, and any lost packet.
pub async fn stop_gracefully(reason: &str) -> RemoteStatus {
    notify_mobile_disconnect(reason).await;
    stop()
}

pub async fn notify_mobile_disconnect(reason: &str) {
    let Some((client, pair_id, bridge_instance_id)) = STATE.lock().unwrap().as_ref().map(|state| {
        (
            state.client.clone(),
            state.pair_id.clone(),
            state.bridge_instance_id.clone(),
        )
    }) else {
        return;
    };
    let payload = serde_json::to_vec(&json!({
        "online": false,
        "disconnected": true,
        "reason": reason,
        "pairId": pair_id,
        "bridgeInstanceId": bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
    }))
    .unwrap_or_default();
    let send = async {
        if client
            .publish(format!("p.{pair_id}.presence"), payload.into())
            .await
            .is_ok()
        {
            let _ = client.flush().await;
        }
    };
    let _ = tokio::time::timeout(DISCONNECT_NOTICE_TIMEOUT, send).await;
}

fn stop_runtime() -> RemoteStatus {
    if let Some(state) = STATE.lock().unwrap().take() {
        let pair_id = state.pair_id.clone();
        let client = state.client.clone();
        tauri::async_runtime::spawn(async move {
            let subject = format!("p.{pair_id}.presence");
            let payload = serde_json::to_vec(&json!({
                "online": false,
                "pairId": pair_id,
                "bridgeInstanceId": state.bridge_instance_id.clone(),
                "lastHeartbeatTs": unix_timestamp(),
            }))
            .unwrap_or_default();
            let _ = client.publish(subject, payload.into()).await;
            let _ = client.flush().await;
        });
        state.event_task.abort();
        state.cmd_task.abort();
        state.transfer_task.abort();
        state.heartbeat_task.abort();
        state.refresh_task.abort();
        if let Some(web_task) = state.web_task {
            web_task.abort();
        }
        transfer::clear_all();
    }
    empty()
}

pub fn status() -> RemoteStatus {
    match STATE.lock().unwrap().as_ref() {
        Some(s) => {
            // Derive real health instead of reporting `connected: true` for as
            // long as STATE is occupied: the NATS client reconnects with state
            // transitions, and every critical background task can die
            // independently. Any finished task requires a full generation
            // reconnect; otherwise the bridge can look connected while losing
            // commands, events, presence, transfers, or credential refreshes.
            let critical_task_dead = s.cmd_task.is_finished()
                || s.transfer_task.is_finished()
                || s.event_task.is_finished()
                || s.heartbeat_task.is_finished()
                || s.refresh_task.is_finished();
            if !critical_task_dead {
                RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
            }
            let reconnect_in_flight = RUNTIME_RECONNECT_RUNNING.load(Ordering::Acquire);
            let can_auto_reconnect =
                RUNTIME_RECONNECT_ATTEMPTS.load(Ordering::Acquire) < MAX_RUNTIME_RECONNECT_ATTEMPTS;
            let reconnecting = critical_task_dead
                && START_REQUESTED.load(Ordering::Acquire)
                && (reconnect_in_flight || can_auto_reconnect);
            if reconnecting && !reconnect_in_flight {
                spawn_runtime_reconnect();
            }
            let connected = !critical_task_dead
                && s.client.connection_state() == async_nats::connection::State::Connected;

            // The local Web listener is optional and retries independently so
            // a busy port never disrupts the healthy phone bridge.
            let web_dead = s.web_task.as_ref().is_none_or(|task| task.is_finished());
            if !web_dead {
                WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
            }
            let web_reconnect_in_flight = WEB_RECONNECT_RUNNING.load(Ordering::Acquire);
            let can_reconnect_web =
                WEB_RECONNECT_ATTEMPTS.load(Ordering::Acquire) < MAX_WEB_RECONNECT_ATTEMPTS;
            if web_dead && can_reconnect_web && !web_reconnect_in_flight {
                spawn_web_reconnect(s.pair_id.clone());
            }
            let web_reconnect_exhausted =
                web_dead && !web_reconnect_in_flight && !can_reconnect_web;
            // Re-expose the pairing code until it expires so the UI keeps it
            // after navigating away and back (it's no longer a show-once value).
            let confirmed = s.pairing_confirmed.load(Ordering::Acquire);
            let code_fresh = !confirmed
                && s.pairing_code.is_some()
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
                reconnecting,
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                pairing_code,
                pairing_code_expires_at,
                desktop_id: s.desktop_id.clone(),
                desktop_public_key: s.desktop_public_key.clone(),
                web_url: s.web_url.clone(),
                web_lan_url: s.web_lan_url.clone(),
                error_code: if critical_task_dead && !reconnecting {
                    Some("reconnect_required".to_string())
                } else if web_reconnect_exhausted {
                    Some("web_bind".to_string())
                } else {
                    None
                },
                error: None,
            }
        }
        // A bridge that stopped on its own (revoked pairing) explains itself
        // through the last recorded error code instead of a bare "not running".
        // When stopped, surface the persisted pair_id so the frontend can still
        // show the paired row (disconnected state) — the authoritative pairing
        // fact is the persisted credential, not the runtime STATE.
        None => {
            let error_code = LAST_ERROR_CODE.lock().unwrap().clone();
            // Startup retries run before a bridge instance exists, so this
            // state cannot be inferred from `STATE`. Expose it explicitly so
            // the UI shows an amber reconnecting indicator instead of briefly
            // presenting the initial transient network/server error as final.
            let reconnecting = START_REQUESTED.load(Ordering::Acquire)
                && START_RETRY_RUNNING.load(Ordering::Acquire)
                && matches!(error_code.as_deref(), Some("network") | Some("server"));
            RemoteStatus {
                reconnecting,
                error_code: if reconnecting { None } else { error_code },
                pair_id: pairing::load_creds().map(|c| c.pair_id).unwrap_or_default(),
                ..empty()
            }
        }
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
/// The drain publishes via core NATS (fire-and-forget). Completeness is
/// guaranteed at the application layer: the client recovers gaps via
/// `get_events_since` backfill on reattach or jitter-gap detection.
///
/// On queue overflow the newest event is dropped and logged; the client heals
/// the gap via `get_events_since` backfill on its next reattach.
#[allow(clippy::too_many_arguments)]
pub fn publish_event(
    session_id: &str,
    event_type: &str,
    data: &str,
    run_id: &str,
    idx: i64,
    epoch: i64,
    event_id: &str,
    timestamp: &str,
    session_idx: i64,
    run_sequence: i64,
) {
    let Some((tx, pair_id, connected)) = ({
        let guard = STATE.lock().unwrap();
        guard.as_ref().map(|s| {
            (
                s.event_tx.clone(),
                s.pair_id.clone(),
                s.client.connection_state() == async_nats::connection::State::Connected,
            )
        })
    }) else {
        return;
    };
    // NATS offline (server down / network out): publish would block until
    // reconnect and queue events that can't be sent. Skip them here — the
    // client recovers any gap via `get_events_since` backfill on its next
    // reattach, same as a dropped event.
    if !connected {
        if let Some(line) = DROP_COUNTERS.record_drop(
            "NATS not connected",
            event_type,
            session_id,
            unix_timestamp_ms(),
        ) {
            eprintln!("{line}");
        }
        return;
    }
    // Guard the NATS payload cap: an oversized event is published with a
    // truncated `data` marker (type/runId/idx preserved) rather than dropped,
    // so the client's dedup cursor doesn't get a permanent hole.
    let body = build_event_body(
        session_id,
        event_type,
        data,
        run_id,
        idx,
        epoch,
        event_id,
        timestamp,
        session_idx,
        run_sequence,
    );
    // A serde_json::Value always serializes, so this cannot fail.
    let payload = serde_json::to_vec(&body).expect("an event Value always serializes");
    let event = EventPublish {
        subject: format!("p.{pair_id}.evt.{session_id}"),
        payload,
    };
    if tx.try_send(event).is_err() {
        if let Some(line) = DROP_COUNTERS.record_drop(
            "event publish queue full",
            event_type,
            session_id,
            unix_timestamp_ms(),
        ) {
            eprintln!("{line}");
        }
        return;
    }
    // A drop episode (queue full or NATS offline) has recovered: this event
    // enqueued normally. Report once, with the episode's total. Best-effort —
    // a burst racing across threads may miscount a line, never the flood.
    if let Some(line) = DROP_COUNTERS.report_recovery() {
        eprintln!("{line}");
    }
}

#[allow(clippy::too_many_arguments)]
fn build_event_body(
    session_id: &str,
    event_type: &str,
    data: &str,
    run_id: &str,
    idx: i64,
    epoch: i64,
    event_id: &str,
    timestamp: &str,
    session_idx: i64,
    run_sequence: i64,
) -> serde_json::Value {
    let data = cap_event_data(data);
    json!({
        "schemaVersion": 2,
        "sessionId": session_id,
        "type": event_type,
        "data": data,
        "runId": run_id,
        "idx": idx,
        "epoch": epoch,
        "eventId": event_id,
        "timestamp": timestamp,
        "sessionIdx": session_idx,
        "runSequence": run_sequence,
    })
}

/// Mirror a run's projection snapshot as a wholesale-replacement signal. The
/// snapshot's folded events cannot be applied incrementally — a coalesced
/// chunk's payload spans idx values the client already applied — so this goes
/// out as a single `run_snapshot` event and the client heals by resyncing
/// rather than folding. The folded events ride along in `data` for consumers
/// that can apply a snapshot directly.
pub fn publish_snapshot(
    session_id: &str,
    run_id: &str,
    snapshot_cursor: i64,
    snapshot_events: &[crate::agent_proto::ProjectedRunEvent],
    run_sequence: i64,
) {
    let events: Vec<serde_json::Value> = snapshot_events
        .iter()
        .map(|event| {
            json!({
                "type": event.r#type,
                "data": future_rpc::decode::projected_event_data(event),
                "idx": event.idx,
            })
        })
        .collect();
    let data = json!({ "snapshotEvents": events, "snapshotCursor": snapshot_cursor }).to_string();
    publish_event(
        session_id,
        "run_snapshot",
        &data,
        run_id,
        snapshot_cursor,
        0,
        &format!("{session_id}:{run_id}:snapshot:{snapshot_cursor}"),
        "",
        -1,
        run_sequence,
    );
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
/// aborted — it keeps its client clone alive until its backlog is flushed,
/// avoiding a mid-stream gap at the swap point.
///
/// Drop/backlog reporting happens in [`publish_event`], event-driven, since
/// this loop can be blocked on `publish().await` (queue-backed NATS) and
/// wouldn't reach its own timer while the backlog persists.
fn spawn_event_publisher(
    client: async_nats::Client,
    mut rx: tokio::sync::mpsc::Receiver<EventPublish>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Err(error) = client.publish(event.subject, event.payload.into()).await {
                eprintln!("remote: event publish failed: {error}");
            }
        }
    })
}

/// Heartbeat cadence. Tests shrink it to milliseconds so the publish pattern
/// (baseline → signature change → self-heal) can be observed without a
/// multi-second wall-clock wait.
fn presence_tick() -> std::time::Duration {
    #[cfg(test)]
    const TICK: std::time::Duration = std::time::Duration::from_millis(10);
    #[cfg(not(test))]
    const TICK: std::time::Duration = std::time::Duration::from_secs(1);
    TICK
}

/// Credential-refresh / health-check cadence. Tests shrink it to milliseconds
/// so the generation swap can run without a 15s wall-clock wait per tick.
fn refresh_tick() -> std::time::Duration {
    #[cfg(test)]
    const TICK: std::time::Duration = std::time::Duration::from_millis(10);
    #[cfg(not(test))]
    const TICK: std::time::Duration = std::time::Duration::from_secs(15);
    TICK
}

fn spawn_presence_heartbeat(
    client: async_nats::Client,
    pair_id: String,
    bridge_instance_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Three independent publish channels:
        //   p.{pair}.presence          — liveness micro-packet every 1s
        //   p.{pair}.state.sessions    — session list on signature change + 20s self-heal
        //   p.{pair}.state.workspaces  — workspace list on dirty + 20s self-heal
        let mut interval = tokio::time::interval(presence_tick());
        let mut last_sessions_sig = String::new();
        let mut last_workspaces_sig = String::new();
        let mut secs_since_sessions: u8 = 20; // first tick publishes a baseline
        let mut secs_since_workspaces: u8 = 20;
        loop {
            interval.tick().await;

            // 1. Liveness micro-packet (every tick). A Value always serializes.
            let bytes = serde_json::to_vec(&light_presence_payload(&pair_id, &bridge_instance_id))
                .expect("a presence Value always serializes");
            if let Err(e) = client
                .publish(format!("p.{pair_id}.presence"), bytes.into())
                .await
            {
                eprintln!("remote: presence heartbeat write failed: {e}");
            }

            // 2. Sessions snapshot (signature change or 20s self-heal).
            let dirty = crate::store::take_catalog_dirty();
            // A prolonged store read failure must not overflow and panic the
            // heartbeat task in debug/dev builds; the task supervisor would
            // reconnect it, but the deterministic panic would simply repeat.
            secs_since_sessions = secs_since_sessions.saturating_add(1);
            secs_since_workspaces = secs_since_workspaces.saturating_add(1);
            if let Some((sessions_payload, sessions_sig)) = build_sessions_snapshot(&pair_id) {
                if sessions_sig != last_sessions_sig || secs_since_sessions >= 20 {
                    let bytes = serde_json::to_vec(&sessions_payload)
                        .expect("a sessions Value always serializes");
                    if let Err(e) = client
                        .publish(format!("p.{pair_id}.state.sessions"), bytes.into())
                        .await
                    {
                        eprintln!("remote: state.sessions publish failed: {e}");
                    }
                    last_sessions_sig = sessions_sig;
                    secs_since_sessions = 0;
                }
            }

            // 3. Workspaces snapshot (dirty flag or 20s self-heal).
            let (workspaces_payload, workspaces_sig) = build_workspaces_snapshot();
            if dirty || workspaces_sig != last_workspaces_sig || secs_since_workspaces >= 20 {
                let bytes = serde_json::to_vec(&workspaces_payload)
                    .expect("a workspaces Value always serializes");
                if let Err(e) = client
                    .publish(format!("p.{pair_id}.state.workspaces"), bytes.into())
                    .await
                {
                    eprintln!("remote: state.workspaces publish failed: {e}");
                }
                last_workspaces_sig = workspaces_sig;
                secs_since_workspaces = 0;
            }
        }
    })
}

fn spawn_credential_refresh(
    pair_id: String,
    reply_slots: commands::ReplySlots,
    pairing_confirmed: Arc<AtomicBool>,
    handshake_state: commands::HandshakeState,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            while !pairing_confirmed.load(Ordering::Acquire) {
                tokio::time::sleep(presence_tick()).await;
            }
            let Some(creds) = pairing::load_creds().filter(|creds| creds.pair_id == pair_id) else {
                return;
            };
            // Tick instead of sleeping until the refresh deadline: the same
            // generation swap that rotates the JWT also heals a dead
            // command/transfer loop or a wedged connection (status() derives
            // real health), so the bridge recovers without a manual
            // stop/start. Two consecutive unhealthy ticks (30s) debounce
            // transient NATS reconnects.
            let mut unhealthy_ticks = 0u8;
            loop {
                tokio::time::sleep(refresh_tick()).await;
                if status().connected {
                    unhealthy_ticks = 0;
                } else {
                    unhealthy_ticks += 1;
                }
                // Refresh this far ahead of expiry (a production tick); kept
                // independent of the test-shrunk tick so the due path stays
                // reachable under test timing.
                let refresh_due =
                    pairing::refresh_delay(&creds) < std::time::Duration::from_secs(15);
                if refresh_due || unhealthy_ticks >= 2 {
                    if unhealthy_ticks >= 2 {
                        eprintln!(
                            "remote: bridge unhealthy for {}s; swapping connection",
                            unhealthy_ticks as u64 * refresh_tick().as_secs()
                        );
                    }
                    break;
                }
            }
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
                    *LAST_ERROR_CODE.lock().unwrap() = Some("revoked".to_string());
                    let _ = pairing::clear_creds();
                    let _ = stop();
                    return;
                }
                Err(error) => {
                    eprintln!("remote: credential refresh failed: {error}");
                    tokio::time::sleep(refresh_tick()).await;
                    continue;
                }
            };
            let client = match connect_nats(&refreshed).await {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("remote: reconnect with refreshed credential failed: {error}");
                    tokio::time::sleep(refresh_tick()).await;
                    continue;
                }
            };
            let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
            let new_event = spawn_event_publisher(client.clone(), event_rx);
            let new_cmd = tokio::spawn(commands::command_loop(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
                handshake_state.clone(),
            ));
            let new_transfer = transfer::spawn_transfer_loop(
                client.clone(),
                pair_id.clone(),
                handshake_state.active_flag(),
            );
            let new_heartbeat = spawn_presence_heartbeat(
                client.clone(),
                pair_id.clone(),
                handshake_state.bridge_instance_id().to_string(),
            );
            // Hold the STATE lock across the generation check AND the creds
            // save: saving outside the lock raced `unpair()` (stop → clear
            // creds) and could resurrect a just-revoked credential file.
            let mut guard = STATE.lock().unwrap();
            let Some(state) = guard.as_mut().filter(|state| state.pair_id == pair_id) else {
                new_event.abort();
                new_cmd.abort();
                new_transfer.abort();
                new_heartbeat.abort();
                return;
            };
            if let Err(error) = pairing::save_creds(&refreshed) {
                eprintln!("remote: save refreshed credential failed: {error}");
            }
            let old_cmd = std::mem::replace(&mut state.cmd_task, new_cmd);
            let old_transfer = std::mem::replace(&mut state.transfer_task, new_transfer);
            let old_heartbeat = std::mem::replace(&mut state.heartbeat_task, new_heartbeat);
            let old_event = std::mem::replace(&mut state.event_task, new_event);
            state.event_tx = event_tx;
            state.client = client;
            state.nats_url = refreshed.nats_url;
            old_cmd.abort();
            old_transfer.abort();
            old_heartbeat.abort();
            // The old event drain is deliberately NOT aborted: dropping the
            // handle detaches it, and it exits on its own after flushing its
            // backlog — no event gap at the swap point.
            drop(old_event);
        }
    })
}

/// Append one signature field as `<byte-len>:<bytes>`. Because every record
/// emits a fixed number of fields in a fixed order, length-prefixing makes the
/// whole catalog signature unambiguous without any record/field separator — so a
/// title that happens to contain a separator character can't collide two
/// different catalogs into the same signature (which would silently skip a sync).
fn push_sig_field(sig: &mut String, value: &str) {
    sig.push_str(&value.len().to_string());
    sig.push(':');
    sig.push_str(value);
}

/// Build the full presence snapshot (directory + per-session streaming) together
/// with a signature that changes iff the snapshot's UI-visible content changes.
/// The signature is recomputed straight from the store each call, so it can never
/// drift from reality: a missed dirty-mark only delays propagation (the 20s
/// heartbeat recomputes and self-heals), it never desyncs.
fn build_presence_snapshot(pair_id: &str, bridge_instance_id: &str) -> (serde_json::Value, String) {
    let active_sessions: Vec<String> = crate::store::active_run_sessions().unwrap_or_default();
    let threads = crate::store::list_threads().unwrap_or_default();
    let workspaces = crate::store::list_workspaces().unwrap_or_default();

    let thread_ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    let run_infos = crate::store::latest_run_infos(&thread_ids).unwrap_or_default();
    let run_status_by_thread: std::collections::HashMap<&str, &str> = run_infos
        .iter()
        .map(|info| (info.thread_id.as_str(), info.status.as_str()))
        .collect();

    let mut sessions: Vec<serde_json::Value> = Vec::new();
    let mut signature = String::new();
    for t in &threads {
        let Some(sid) = t
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let streaming = active_sessions.iter().any(|active| active == sid);
        let status = run_status_by_thread.get(t.id.as_str()).copied();
        sessions.push(json!({
            "sessionId": sid,
            "threadId": t.id,
            "title": t.title,
            "mode": t.mode,
            "workspaceId": t.workspace_id,
            "pinned": t.pinned,
            "streaming": streaming,
            "status": status,
        }));
        signature.push('s');
        push_sig_field(&mut signature, sid);
        push_sig_field(&mut signature, &t.id);
        push_sig_field(&mut signature, &t.title);
        push_sig_field(&mut signature, &t.mode);
        push_sig_field(&mut signature, &t.workspace_id);
        push_sig_field(&mut signature, if t.pinned { "1" } else { "0" });
        push_sig_field(&mut signature, if streaming { "1" } else { "0" });
        push_sig_field(&mut signature, status.unwrap_or(""));
    }

    let mut workspace_values: Vec<serde_json::Value> = Vec::new();
    for w in &workspaces {
        if w.kind != "user" {
            continue;
        }
        if let Ok(value) = serde_json::to_value(w) {
            workspace_values.push(value);
        }
        signature.push('w');
        push_sig_field(&mut signature, &w.id);
        push_sig_field(&mut signature, &w.name);
    }

    let payload = json!({
        "online": true,
        "pairId": pair_id,
        "bridgeInstanceId": bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
        "sessions": sessions,
        "workspaces": workspace_values,
    });
    (payload, signature)
}

/// Full directory snapshot for the handshake and on-demand `get_presence`
/// (always complete, so a freshly connected client gets a usable baseline).
fn build_presence_payload(pair_id: &str, bridge_instance_id: &str) -> serde_json::Value {
    build_presence_snapshot(pair_id, bridge_instance_id).0
}

/// Sessions-only snapshot for `p.{pair}.state.sessions`.
///
/// Returns `None` when any backing store read fails. A transient SQLite error
/// must not surface as an *empty* session list — publishing `{"sessions":[]}`
/// makes the phone's "selected session vanished" heuristic fire and close a
/// conversation the user is reading (audit 05 L8). On failure the publisher
/// skips this tick and the phone keeps the previous snapshot.
fn build_sessions_snapshot(pair_id: &str) -> Option<(serde_json::Value, String)> {
    let active_sessions = crate::store::active_run_sessions().ok()?;
    let threads = crate::store::list_threads().ok()?;

    let thread_ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    let run_infos = crate::store::latest_run_infos(&thread_ids).ok()?;
    let run_status_by_thread: std::collections::HashMap<&str, &str> = run_infos
        .iter()
        .map(|info| (info.thread_id.as_str(), info.status.as_str()))
        .collect();

    let mut sessions: Vec<serde_json::Value> = Vec::new();
    let mut signature = String::new();
    for t in &threads {
        let Some(sid) = t
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let streaming = active_sessions.iter().any(|active| active == sid);
        let status = run_status_by_thread.get(t.id.as_str()).copied();
        sessions.push(json!({
            "sessionId": sid,
            "threadId": t.id,
            "title": t.title,
            "mode": t.mode,
            "workspaceId": t.workspace_id,
            "pinned": t.pinned,
            "streaming": streaming,
            "status": status,
        }));
        push_sig_field(&mut signature, sid);
        push_sig_field(&mut signature, &t.id);
        push_sig_field(&mut signature, &t.title);
        push_sig_field(&mut signature, &t.mode);
        push_sig_field(&mut signature, &t.workspace_id);
        push_sig_field(&mut signature, if t.pinned { "1" } else { "0" });
        push_sig_field(&mut signature, if streaming { "1" } else { "0" });
        push_sig_field(&mut signature, status.unwrap_or(""));
    }

    let payload = json!({
        "pairId": pair_id,
        "sessions": sessions,
    });
    Some((payload, signature))
}

/// Workspaces-only snapshot for `p.{pair}.state.workspaces`.
fn build_workspaces_snapshot() -> (serde_json::Value, String) {
    let workspaces = crate::store::list_workspaces().unwrap_or_default();
    let mut workspace_values: Vec<serde_json::Value> = Vec::new();
    let mut signature = String::new();
    for w in &workspaces {
        if w.kind != "user" {
            continue;
        }
        if let Ok(value) = serde_json::to_value(w) {
            workspace_values.push(value);
        }
        push_sig_field(&mut signature, &w.id);
        push_sig_field(&mut signature, &w.name);
    }
    let payload = json!({ "workspaces": workspace_values });
    (payload, signature)
}

/// Liveness-only heartbeat (no directory). Sent every ~20s while the catalog is
/// unchanged so an idle link carries almost no traffic.
fn light_presence_payload(pair_id: &str, bridge_instance_id: &str) -> serde_json::Value {
    json!({
        "online": true,
        "pairId": pair_id,
        "bridgeInstanceId": bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
    })
}

fn unix_timestamp() -> u64 {
    // `unwrap_or_default`: a pre-epoch clock is not a reachable failure mode
    // worth an arm — treat it as 0.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Cap on concurrent accepted web-client connections. Acquired BEFORE `accept`
/// so a flood of idle sockets can't exhaust file descriptors (the accept loop
/// blocks at capacity instead of parking unbounded tasks).
const WEB_MAX_CONNECTIONS: usize = 32;

/// A client that connects and never sends a request can't hold a task + fd
/// open indefinitely; its read times out and the connection is dropped. Tests
/// shrink the timeout so the silent-client path runs fast.
fn web_read_timeout() -> std::time::Duration {
    #[cfg(test)]
    const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);
    #[cfg(not(test))]
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    TIMEOUT
}

/// `desktop/web/` on disk — one level up from CARGO_MANIFEST_DIR (desktop/src-tauri/).
fn web_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../web")
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web"))
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
/// The probe target is an IPv4 literal, so the selected source address is
/// always IPv4 — no address-family arm is needed.
fn lan_ip() -> Option<String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Serve the web client from `desktop/web/` on the already-bound listener.
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
            // here instead of accepting sockets it can't serve. The semaphore
            // is never closed, so acquisition cannot fail.
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("web connection semaphore is never closed");
            // An accept failure drops the permit with the temporary and tries
            // the next connection.
            if let Ok((stream, _)) = listener.accept().await {
                spawn_web_connection(permit, stream, web_dir.clone());
            }
        }
    })
}

fn spawn_web_connection(
    permit: tokio::sync::OwnedSemaphorePermit,
    mut stream: tokio::net::TcpStream,
    web_dir: std::path::PathBuf,
) {
    tokio::spawn(async move {
        let _permit = permit; // held until the handler returns
        handle_web_request(&mut stream, &web_dir).await;
    });
}

async fn handle_web_request(stream: &mut tokio::net::TcpStream, web_dir: &std::path::Path) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 8192];
    let n = match tokio::time::timeout(web_read_timeout(), stream.read(&mut buf)).await {
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

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn drop_log_rate_limits_episodes() {
        let counters = DropCounters::new();

        // First drop logs the episode start; the next 8 are silent.
        assert!(counters
            .record_drop("queue full", "tool_delta", "s1", 1_000)
            .is_some());
        for i in 2..=9 {
            let line = counters.record_drop("queue full", "tool_delta", "s1", 1_000 + i);
            assert!(line.is_none(), "drop {i} should be rate-limited");
        }

        // 10s after the last line, the 10th drop reports a cumulative total.
        let line = counters
            .record_drop("queue full", "tool_delta", "s1", 12_000)
            .expect("10th drop after 10s reports a total");
        assert!(line.contains("dropped 10 events"), "got: {line}");
    }

    #[test]
    fn drop_log_reports_recovery_and_resets() {
        let counters = DropCounters::new();

        counters.record_drop("NATS not connected", "tool_delta", "s1", 1_000);
        let line = counters
            .report_recovery()
            .expect("recovery after an active episode reports");
        assert!(line.contains("dropped 1 events"), "got: {line}");

        // No episode active: recovery is silent and counters stay zero.
        assert!(counters.report_recovery().is_none());

        // The next episode starts fresh: the first drop logs again.
        assert!(counters
            .record_drop("queue full", "tool_delta", "s1", 2_000)
            .is_some());
    }

    #[test]
    fn nats_v2_event_keeps_every_v1_field_unchanged() {
        let body = build_event_body(
            "session-1",
            "text_chunk",
            r#"{"text":"hi"}"#,
            "run-1",
            7,
            2,
            "evt-1",
            "2026-08-02T00:00:00Z",
            -1,
            11,
        );
        assert_eq!(body["type"], "text_chunk");
        assert_eq!(body["data"], r#"{"text":"hi"}"#);
        assert_eq!(body["runId"], "run-1");
        assert_eq!(body["idx"], 7);
        assert_eq!(body["schemaVersion"], 2);
        assert_eq!(body["sessionId"], "session-1");
        assert_eq!(body["epoch"], 2);
        assert_eq!(body["eventId"], "evt-1");
        assert_eq!(body["runSequence"], 11);
    }

    #[test]
    fn nats_session_event_has_independent_cursor() {
        let body = build_event_body(
            "session-1",
            "model_changed",
            "{}",
            "",
            -1,
            0,
            "session-1:session:4",
            "2026-08-02T00:00:00Z",
            4,
            -1,
        );
        assert_eq!(body["runId"], "");
        assert_eq!(body["sessionIdx"], 4);
        assert_eq!(body["runSequence"], -1);
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::test_support::{
        await_publish, init_store, jwt, nats_connect, nats_connect_once, now_secs, sign_in, unique,
        FakeNats, HomeGuard, MockPlatform,
    };
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn test_creds(pair_id: &str, nats_url: &str, expires_in: i64) -> pairing::PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        pairing::PairingCreds {
            handshake_version: 1,
            pair_id: pair_id.to_string(),
            desktop_id: format!("desktop_{}", unique("rt")),
            nkey_seed: key_pair.seed().unwrap().to_string(),
            user_jwt: jwt(now_secs() + expires_in),
            nats_url: nats_url.to_string(),
            nats_ws_url: nats_url.replace("nats://", "ws://"),
            jwt_expires_at: now_secs() + expires_in,
        }
    }

    /// A RemoteState wired to a live fake NATS server, with placeholder tasks
    /// (pending forever) for the loops this test doesn't drive.
    async fn fake_state(nats: &FakeNats, pair_id: &str) -> RemoteState {
        let client = nats_connect(nats).await;
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let event_task = spawn_event_publisher(client.clone(), event_rx);
        RemoteState {
            client,
            nats_url: nats.url().to_string(),
            pair_id: pair_id.to_string(),
            desktop_id: format!("desktop_{}", unique("rt")),
            desktop_public_key: "UPUBKEY".to_string(),
            bridge_instance_id: format!("bridge_{}", unique("rt")),
            event_tx,
            event_task,
            cmd_task: tokio::spawn(std::future::pending()),
            transfer_task: tokio::spawn(std::future::pending()),
            heartbeat_task: tokio::spawn(std::future::pending()),
            refresh_task: tokio::spawn(std::future::pending()),
            web_task: None,
            web_url: None,
            web_lan_url: None,
            pairing_code: None,
            pairing_code_expires_at: None,
            pairing_confirmed: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Install the state; tests MUST clean up via `stop()` so the next
    /// serialized test starts clean. Poison-tolerant: one test's failure must
    /// not cascade into every later lock.
    fn install_state(state: RemoteState) {
        let previous = STATE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .replace(state);
        assert!(previous.is_none(), "previous test leaked STATE");
    }

    /// Wait until WEB_PORT is bindable again (stop() aborts the web task
    /// asynchronously — the socket lingers briefly).
    async fn wait_for_web_port_free() {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(listener) = std::net::TcpListener::bind(("0.0.0.0", WEB_PORT)) {
                drop(listener);
                return;
            }
            assert!(std::time::Instant::now() < deadline, "web port never freed");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn status_of_stopped_bridge_surfaces_last_error_and_pairing() {
        let _home = HomeGuard::new("remote-stopped");
        assert!(STATE.lock().unwrap().is_none());
        let current = status();
        assert!(!current.running);
        assert!(!current.connected);
        assert_eq!(current.error_code, None);
        assert_eq!(current.pair_id, "");

        pairing::save_creds(&test_creds("pair_stopped", "nats://127.0.0.1:1", 3600)).unwrap();
        *LAST_ERROR_CODE.lock().unwrap() = Some("revoked".to_string());
        let current = status();
        assert_eq!(current.error_code.as_deref(), Some("revoked"));
        assert_eq!(current.pair_id, "pair_stopped");

        // A transient first connect failure starts the process-lifetime retry
        // worker before `STATE` exists. It is reconnecting, not a final error.
        *LAST_ERROR_CODE.lock().unwrap() = Some("network".to_string());
        START_REQUESTED.store(true, Ordering::Release);
        START_RETRY_RUNNING.store(true, Ordering::Release);
        let current = status();
        assert!(current.reconnecting);
        assert_eq!(current.error_code, None);

        START_RETRY_RUNNING.store(false, Ordering::Release);
        START_REQUESTED.store(false, Ordering::Release);
        *LAST_ERROR_CODE.lock().unwrap() = None;
        pairing::clear_creds().unwrap();
    }

    #[test]
    fn stop_without_state_is_an_empty_noop() {
        let _home = HomeGuard::new("remote-stop-empty");
        let stopped = stop();
        assert!(!stopped.running);
    }

    #[tokio::test]
    async fn status_of_running_bridge_reports_health_and_pairing_code() {
        let _home = HomeGuard::new("remote-status");
        let nats = FakeNats::start().await;
        let mut state = fake_state(&nats, "pair_status").await;
        state.web_task = Some(tokio::spawn(std::future::pending()));
        state.web_url = Some("http://localhost:8022".to_string());
        state.pairing_code = Some("code-123".to_string());
        state.pairing_code_expires_at = Some(now_secs() + 600);
        state.pairing_confirmed = Arc::new(AtomicBool::new(false));
        install_state(state);

        // Unconfirmed + fresh code → the code is re-exposed.
        let current = status();
        assert!(current.running);
        assert!(current.connected);
        assert_eq!(current.pairing_code.as_deref(), Some("code-123"));
        assert_eq!(current.error_code, None);

        // Expired code → hidden even while unconfirmed.
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.pairing_code_expires_at = Some(now_secs() - 1);
        }
        assert_eq!(status().pairing_code, None);

        // Confirmed → hidden regardless of freshness.
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.pairing_code_expires_at = Some(now_secs() + 600);
            state.pairing_confirmed.store(true, Ordering::Release);
        }
        assert_eq!(status().pairing_code, None);

        // A dead critical task first enters transparent automatic reconnect.
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.cmd_task.abort();
            state.cmd_task = tokio::spawn(async {});
        }
        START_REQUESTED.store(true, Ordering::Release);
        RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let current = status();
        assert!(!current.connected);
        assert!(current.reconnecting);
        assert_eq!(current.error_code, None);

        // Only after the bounded automatic budget is exhausted does the UI
        // receive an actionable reconnect-required error.
        RUNTIME_RECONNECT_ATTEMPTS.store(MAX_RUNTIME_RECONNECT_ATTEMPTS, Ordering::Release);
        let current = status();
        assert!(!current.reconnecting);
        assert_eq!(current.error_code.as_deref(), Some("reconnect_required"));

        // The optional Web listener retries silently, then becomes actionable
        // only after its own reconnect budget is exhausted.
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.web_task = None;
            state.cmd_task = tokio::spawn(std::future::pending());
        }
        assert_eq!(status().error_code, None);
        WEB_RECONNECT_ATTEMPTS.store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        assert_eq!(status().error_code.as_deref(), Some("web_bind"));

        let stopped = stop();
        assert!(!stopped.running);
        assert!(STATE.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn publish_event_mirrors_caps_and_reports_drops() {
        let _home = HomeGuard::new("remote-publish");
        let nats = FakeNats::start().await;
        let state = fake_state(&nats, "pair_pub").await;
        install_state(state);
        let mut tap = nats.tap();

        publish_event(
            "sess-a",
            "text_chunk",
            r#"{"text":"hi"}"#,
            "run-1",
            3,
            1,
            "e1",
            "",
            -1,
            7,
        );
        let published =
            await_publish(&mut tap, "p.pair_pub.evt.sess-a", Duration::from_secs(5)).await;
        let body = published.json();
        assert_eq!(body["type"], json!("text_chunk"));
        assert_eq!(body["data"], json!(r#"{"text":"hi"}"#));
        assert_eq!(body["idx"], json!(3));

        // An oversized event keeps its identity but ships a truncation marker
        // (as the `data` string payload).
        let huge = "x".repeat(MAX_EVENT_BYTES + 10);
        publish_event(
            "sess-a",
            "tool_delta",
            &huge,
            "run-1",
            4,
            1,
            "e2",
            "",
            -1,
            8,
        );
        let published =
            await_publish(&mut tap, "p.pair_pub.evt.sess-a", Duration::from_secs(5)).await;
        let body = published.json();
        assert_eq!(body["type"], json!("tool_delta"));
        assert_eq!(body["idx"], json!(4));
        let data = body["data"].as_str().expect("truncated marker is a string");
        assert!(data.contains("\"_truncated\":true"), "got: {data}");

        // Snapshots ride the same channel as run_snapshot events.
        publish_snapshot(
            "sess-a",
            "run-1",
            9,
            &[crate::agent_proto::ProjectedRunEvent {
                r#type: "text_chunk".to_string(),
                data: r#"{"text":"folded"}"#.to_string(),
                idx: 2,
                payload: None,
            }],
            7,
        );
        let published =
            await_publish(&mut tap, "p.pair_pub.evt.sess-a", Duration::from_secs(5)).await;
        let body = published.json();
        assert_eq!(body["type"], json!("run_snapshot"));
        let data: serde_json::Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
        assert_eq!(data["snapshotCursor"], json!(9));
        assert_eq!(data["snapshotEvents"][0]["idx"], json!(2));

        stop();
    }

    #[tokio::test]
    async fn publish_event_reports_offline_and_full_queue_drops() {
        let _home = HomeGuard::new("remote-drops");
        // The drop counters are process-global: start from a clean slate so an
        // episode leaked by another test can't suppress the first-drop line.
        DROP_COUNTERS.dropping.store(false, Ordering::Relaxed);
        DROP_COUNTERS.dropped.store(0, Ordering::Relaxed);
        DROP_COUNTERS.last_report.store(0, Ordering::Relaxed);
        // No state at all → immediate return.
        publish_event("sess", "t", "{}", "r", 0, 0, "e", "", -1, 0);

        let nats = FakeNats::start().await;
        let state = fake_state(&nats, "pair_drops").await;
        install_state(state);
        // Kill the broker and wait for the client to notice.
        nats.kill();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let disconnected = {
                let guard = STATE.lock().unwrap();
                let state = guard.as_ref().unwrap();
                state.client.connection_state() != async_nats::connection::State::Connected
            };
            if disconnected {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "client never noticed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Offline → the drop-reporting path (episode start).
        publish_event("sess", "t", "{}", "r", 0, 0, "e", "", -1, 0);

        // Full/closed queue → the queue-full drop path. Reset the episode so
        // this drop (not the offline one above) is the "first" and gets a line.
        DROP_COUNTERS.dropping.store(false, Ordering::Relaxed);
        DROP_COUNTERS.dropped.store(0, Ordering::Relaxed);
        DROP_COUNTERS.last_report.store(0, Ordering::Relaxed);
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            drop(rx);
            state.event_tx = tx;
        }
        // Fake a connected client so the queue path (not the offline path) runs.
        let nats2 = FakeNats::start().await;
        let client2 = nats_connect(&nats2).await;
        {
            let mut guard = STATE.lock().unwrap();
            guard.as_mut().unwrap().client = client2;
        }
        publish_event("sess", "t", "{}", "r", 1, 0, "e", "", -1, 0);

        // Recovery: a successful enqueue after the episode reports once.
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let drain =
            spawn_event_publisher(STATE.lock().unwrap().as_ref().unwrap().client.clone(), rx);
        {
            let mut guard = STATE.lock().unwrap();
            guard.as_mut().unwrap().event_tx = tx;
        }
        let mut tap = nats2.tap();
        publish_event("sess", "t", "{}", "r", 2, 0, "e", "", -1, 0);
        await_publish(&mut tap, "p.pair_drops.evt.sess", Duration::from_secs(5)).await;

        drain.abort();
        stop();
    }

    #[tokio::test]
    async fn event_publisher_flushes_in_order_and_reports_failures() {
        let _home = HomeGuard::new("remote-drain");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let drain = spawn_event_publisher(client.clone(), rx);
        let mut tap = nats.tap();

        for index in 0..3 {
            tx.send(EventPublish {
                subject: format!("p.pair_drain.evt.sess.{index}"),
                payload: format!("{{\"n\":{index}}}").into_bytes(),
            })
            .await
            .unwrap();
        }
        for index in 0..3 {
            let published = await_publish(
                &mut tap,
                &format!("p.pair_drain.evt.sess.{index}"),
                Duration::from_secs(5),
            )
            .await;
            assert_eq!(published.json()["n"], json!(index));
        }

        // An event over the broker's max_payload cap is refused client-side;
        // the publisher logs the failure and keeps draining the queue.
        tx.send(EventPublish {
            subject: "p.pair_drain.evt.sess.huge".to_string(),
            payload: vec![b'x'; 9 * 1024 * 1024],
        })
        .await
        .unwrap();
        tx.send(EventPublish {
            subject: "p.pair_drain.evt.sess.after".to_string(),
            payload: b"{}".to_vec(),
        })
        .await
        .unwrap();
        await_publish(
            &mut tap,
            "p.pair_drain.evt.sess.after",
            Duration::from_secs(5),
        )
        .await;

        // Publishing after the broker dies logs the failure and keeps draining.
        nats.kill();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while client.connection_state() == async_nats::connection::State::Connected {
            assert!(std::time::Instant::now() < deadline, "client never noticed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tx.send(EventPublish {
            subject: "p.pair_drain.evt.sess.late".to_string(),
            payload: b"{}".to_vec(),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Closing the channel ends the drain task.
        drop(tx);
        tokio::time::timeout(Duration::from_secs(5), drain)
            .await
            .expect("drain completes")
            .expect("drain not panicked");
    }

    #[tokio::test]
    async fn unpair_revokes_and_clears() {
        let _home = HomeGuard::new("remote-unpair");
        // No creds, not running → plain success.
        unpair().await.unwrap();

        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_unpair", "nats://127.0.0.1:1", 3600);
        pairing::save_creds(&creds).unwrap();
        platform.push("/client/v1/remote/pair/revoke", 200, json!({}));
        unpair().await.unwrap();
        assert!(pairing::load_creds().is_none());
        assert_eq!(platform.requests().len(), 1);

        // A revoke failure propagates.
        pairing::save_creds(&creds).unwrap();
        platform.push(
            "/client/v1/remote/pair/revoke",
            500,
            json!({ "error": "boom", "message": "no" }),
        );
        assert!(unpair().await.is_err());
        pairing::clear_creds().unwrap();
    }

    #[tokio::test]
    async fn start_runs_the_full_bridge_and_stop_winds_it_down() {
        let _home = HomeGuard::new("remote-start");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.respond_pair_code(nats.url());

        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.running);
        assert!(started.connected);
        assert!(started.pairing_code.is_some());
        assert_eq!(started.web_url.as_deref(), Some("http://localhost:8022"));
        assert_eq!(started.error_code, None);

        let mut tap = nats.tap();
        // The presence heartbeat and both catalog snapshots flow.
        let presence = await_publish(
            &mut tap,
            &format!("p.{}.presence", started.pair_id),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(presence.json()["online"], json!(true));
        await_publish(
            &mut tap,
            &format!("p.{}.state.sessions", started.pair_id),
            Duration::from_secs(5),
        )
        .await;
        await_publish(
            &mut tap,
            &format!("p.{}.state.workspaces", started.pair_id),
            Duration::from_secs(5),
        )
        .await;

        // The event mirror is live.
        publish_event(
            "sess-live",
            "text_chunk",
            "{}",
            "run-1",
            1,
            0,
            "e",
            "",
            -1,
            0,
        );
        await_publish(
            &mut tap,
            &format!("p.{}.evt.sess-live", started.pair_id),
            Duration::from_secs(5),
        )
        .await;

        // The web client serves over HTTP.
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", WEB_PORT))
            .await
            .expect("web server accepts");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
        assert!(response.contains("text/html"), "got: {response}");

        // status() re-exposes the unexpired, unconfirmed pairing code.
        let polled = status();
        assert!(polled.pairing_code.is_some());

        // stop() publishes offline presence and clears the state.
        stop();
        let offline = await_publish(
            &mut tap,
            &format!("p.{}.presence", started.pair_id),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(offline.json()["online"], json!(false));
        assert!(STATE.lock().unwrap().is_none());
        wait_for_web_port_free().await;
    }

    #[tokio::test]
    async fn start_retries_web_bind_before_reporting_failure() {
        let _home = HomeGuard::new("remote-web-bind");
        wait_for_web_port_free().await;
        let blocker =
            std::net::TcpListener::bind(("0.0.0.0", WEB_PORT)).expect("occupy the web port");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.respond_pair_code(nats.url());

        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.running);
        assert_eq!(started.web_url, None);
        assert_eq!(started.web_lan_url, None);
        assert_eq!(started.error_code, None);
        assert_eq!(status().error_code, None);
        WEB_RECONNECT_ATTEMPTS.store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        assert_eq!(status().error_code.as_deref(), Some("web_bind"));
        stop();
        drop(blocker);
    }

    #[tokio::test]
    async fn start_with_existing_credentials_refreshes_instead_of_pairing() {
        let _home = HomeGuard::new("remote-start-refresh");
        init_store();
        wait_for_web_port_free().await;
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_keep", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());

        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.running);
        assert_eq!(started.pairing_code, None, "existing pairing → no new code");
        assert!(pairing::load_creds().is_some(), "refreshed creds persisted");
        stop();
    }

    #[tokio::test]
    async fn start_replaces_revoked_and_legacy_credentials() {
        let _home = HomeGuard::new("remote-start-revoked");
        init_store();
        wait_for_web_port_free().await;
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());

        // Persisted v1 creds the server has revoked → fresh pairing code.
        let creds = test_creds("pair_revoked", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh_revoked();
        platform.respond_pair_code(nats.url());
        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.pairing_code.is_some());
        stop();
        wait_for_web_port_free().await;

        // Legacy (pre-handshake) creds are dropped and re-paired.
        let mut legacy = test_creds("pair_legacy", nats.url(), 3600);
        legacy.handshake_version = 0;
        pairing::save_creds(&legacy).unwrap();
        platform.respond_pair_code(nats.url());
        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.pairing_code.is_some());
        stop();
    }

    #[tokio::test]
    async fn establish_surfaces_a_transient_refresh_failure() {
        let _home = HomeGuard::new("remote-establish-err");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_estab", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        // A transient (non-revoked) refresh failure propagates to the caller
        // instead of minting a replacement pairing.
        platform.push(
            "/client/v1/remote/auth/token",
            500,
            json!({ "message": "busy" }),
        );
        let error = establish().await.unwrap_err();
        assert!(
            matches!(error, crate::AppError::Remote { status: 500, .. }),
            "expected a transient remote failure, got: {error}"
        );
        // The persisted credential is left untouched for the next attempt.
        assert_eq!(pairing::load_creds().unwrap().pair_id, "pair_estab");
    }

    #[tokio::test]
    async fn start_failures_map_to_localized_status_or_error() {
        let _home = HomeGuard::new("remote-start-fail");

        // Platform unreachable → categorized "network" status, not running.
        sign_in("http://127.0.0.1:9");
        let started = start(RemoteStartInput {})
            .await
            .expect("network maps to status");
        assert!(!started.running);
        assert_eq!(started.error_code.as_deref(), Some("network"));
        // The code sticks for later status() polls.
        assert_eq!(status().error_code.as_deref(), Some("network"));

        // Pairing issued but NATS unreachable → same categorized failure.
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.respond_pair_code("nats://127.0.0.1:9");
        let started = start(RemoteStartInput {})
            .await
            .expect("nats failure maps to status");
        assert_eq!(started.error_code.as_deref(), Some("network"));

        // An uncategorized local failure (the credential directory is
        // read-only, so clearing the legacy pairing fails) propagates as Err.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let nats = FakeNats::start().await;
            sign_in(platform.url());
            let mut legacy = test_creds("pair_dir", nats.url(), 3600);
            legacy.handshake_version = 0;
            pairing::save_creds(&legacy).unwrap();
            let config_dir = crate::home_dir().unwrap();
            let config_dir = std::path::Path::new(&config_dir).join(".future");
            let permissions = std::fs::metadata(&config_dir).unwrap().permissions();
            std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            let result = start(RemoteStartInput {}).await;
            std::fs::set_permissions(&config_dir, permissions).unwrap();
            assert!(result.is_err());
        }
        *LAST_ERROR_CODE.lock().unwrap() = None;
    }

    #[test]
    fn only_transient_remote_start_failures_arm_background_recovery() {
        let network = RemoteStatus {
            error_code: Some("network".to_string()),
            ..empty()
        };
        let server = RemoteStatus {
            error_code: Some("server".to_string()),
            ..empty()
        };
        let revoked = RemoteStatus {
            error_code: Some("revoked".to_string()),
            ..empty()
        };
        assert!(retryable_start_status(&network));
        assert!(retryable_start_status(&server));
        assert!(!retryable_start_status(&revoked));
        assert!(!retryable_start_status(&RemoteStatus {
            running: true,
            error_code: Some("network".to_string()),
            ..empty()
        }));
    }

    #[tokio::test]
    async fn credential_refresh_swaps_generations() {
        let _home = HomeGuard::new("remote-refresh-swap");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let nats2 = FakeNats::start().await;
        sign_in(platform.url());
        // Expiring credential → refresh is due on the first tick.
        let creds = test_creds("pair_swap", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats2.url());

        let state = fake_state(&nats, "pair_swap").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake = commands::HandshakeState::new(
            creds.clone(),
            confirmed.clone(),
            "bridge_swap".to_string(),
        );
        let handle = spawn_credential_refresh(
            "pair_swap".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        // The refresh runs, reconnects to the second server and swaps STATE.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let swapped = {
                let guard = STATE.lock().unwrap();
                guard
                    .as_ref()
                    .map(|state| state.nats_url == nats2.url())
                    .unwrap_or(false)
            };
            if swapped {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "generation never swapped"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // The refreshed credential was persisted under the STATE lock.
        assert_eq!(pairing::load_creds().unwrap().nats_url, nats2.url());
        assert!(platform
            .requests()
            .iter()
            .any(|(_, path, _)| path == "/client/v1/remote/auth/token"));

        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_logs_credential_save_failures() {
        let _home = HomeGuard::new("remote-refresh-savefail");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let nats2 = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_savefail", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        // Two successful refreshes: the first hits the injected save failure,
        // the second proves the loop logged it and kept going.
        platform.respond_refresh(nats2.url());
        platform.respond_refresh(nats2.url());
        pairing::INJECT_SAVE_FAILURE.store(true, Ordering::Relaxed);

        let state = fake_state(&nats, "pair_savefail").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake = commands::HandshakeState::new(
            creds.clone(),
            confirmed.clone(),
            "bridge_savefail".to_string(),
        );
        let handle = spawn_credential_refresh(
            "pair_savefail".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while platform.requests().len() < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "refresh never continued past the save failure"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_stops_the_bridge_when_revoked() {
        let _home = HomeGuard::new("remote-refresh-revoked");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_rev", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh_revoked();

        let state = fake_state(&nats, "pair_rev").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_rev".into());
        let handle = spawn_credential_refresh(
            "pair_rev".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("refresh task ends after revocation")
            .expect("refresh task not panicked");
        assert!(STATE.lock().unwrap().is_none(), "bridge stopped itself");
        assert_eq!(LAST_ERROR_CODE.lock().unwrap().as_deref(), Some("revoked"));
        assert!(pairing::load_creds().is_none());
        *LAST_ERROR_CODE.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn credential_refresh_retries_on_transient_failures() {
        let _home = HomeGuard::new("remote-refresh-retry");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_retry", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        // Transient server error → retry; then refreshes pointing at a dead
        // broker → reconnect failure → retry (scripted twice so the loop is
        // observed completing a full failure iteration, not aborting mid-nap).
        platform.push(
            "/client/v1/remote/auth/token",
            500,
            json!({ "message": "busy" }),
        );
        platform.respond_refresh("nats://127.0.0.1:9");
        platform.respond_refresh("nats://127.0.0.1:9");

        let state = fake_state(&nats, "pair_retry").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_retry".into());
        let handle = spawn_credential_refresh(
            "pair_retry".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        // Both failure arms run and the loop keeps going: a third request
        // proves the dead-broker iteration ran its sleep+continue to the end.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while platform.requests().len() < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "refresh never retried"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_returns_when_the_world_moved_on() {
        let _home = HomeGuard::new("remote-refresh-guards");
        let nats = FakeNats::start().await;

        // No persisted credential → immediate return.
        let creds = test_creds("pair_none", nats.url(), 3600);
        let handshake = commands::HandshakeState::new(
            creds.clone(),
            Arc::new(AtomicBool::new(true)),
            "bridge_none".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_none".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("no-creds refresh returns")
            .unwrap();

        // A persisted credential for a DIFFERENT pairing → return.
        let other = test_creds("pair_other", nats.url(), 3600);
        pairing::save_creds(&other).unwrap();
        let handshake = commands::HandshakeState::new(
            creds.clone(),
            Arc::new(AtomicBool::new(true)),
            "bridge_other".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_none".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("mismatched-pairing refresh returns")
            .unwrap();
        pairing::clear_creds().unwrap();
    }

    #[tokio::test]
    async fn credential_refresh_aborts_when_state_generation_mismatches() {
        let _home = HomeGuard::new("remote-refresh-gen");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_gen", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());

        // STATE belongs to another pairing → the swap is abandoned.
        let state = fake_state(&nats, "pair_someone_else").await;
        install_state(state);
        let handshake = commands::HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(true)),
            "bridge_gen".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_gen".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("mismatched-generation refresh returns")
            .unwrap();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_swaps_after_sustained_unhealthy_ticks() {
        let _home = HomeGuard::new("remote-refresh-unhealthy");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let nats2 = FakeNats::start().await;
        sign_in(platform.url());
        // Far-future expiry: only the unhealthy debounce triggers the swap.
        let creds = test_creds("pair_sick", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats2.url());

        let state = fake_state(&nats, "pair_sick").await;
        install_state(state);
        let handshake = commands::HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(true)),
            "bridge_sick".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_sick".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        // Kill the broker: two unhealthy ticks trigger the generation swap.
        nats.kill();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let swapped = {
                let guard = STATE.lock().unwrap();
                guard
                    .as_ref()
                    .map(|state| state.nats_url == nats2.url())
                    .unwrap_or(false)
            };
            if swapped {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "unhealthy swap never ran"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn heartbeat_publishes_presence_and_catalog_snapshots() {
        let _home = HomeGuard::new("remote-heartbeat");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairhb");

        // A thread (with a live run → streaming) and a user workspace.
        let session = unique("sesshb");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Heartbeat thread".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("HB Workspace".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();

        let mut tap = nats.tap();
        let handle = spawn_presence_heartbeat(client.clone(), pair.clone(), "bridge_hb".into());

        let presence = await_publish(
            &mut tap,
            &format!("p.{pair}.presence"),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(presence.json()["online"], json!(true));
        assert_eq!(presence.json()["pairId"], json!(pair));

        let sessions = await_publish(
            &mut tap,
            &format!("p.{pair}.state.sessions"),
            Duration::from_secs(5),
        )
        .await;
        let rows = sessions.json()["sessions"].as_array().unwrap().clone();
        let row = rows
            .iter()
            .find(|row| row["sessionId"] == json!(session))
            .unwrap();
        assert_eq!(row["streaming"], json!(true));
        assert_eq!(row["title"], json!("Heartbeat thread"));

        let workspaces = await_publish(
            &mut tap,
            &format!("p.{pair}.state.workspaces"),
            Duration::from_secs(5),
        )
        .await;
        let list = workspaces.json()["workspaces"].as_array().unwrap().clone();
        assert!(list.iter().any(|w| w["name"] == json!("HB Workspace")));

        // A catalog change is picked up by the signature check on a later tick.
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed heartbeat".to_string(),
        })
        .unwrap();
        let mut saw_rename = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !saw_rename {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "rename never republished");
            let published = tokio::time::timeout(remaining, tap.recv())
                .await
                .expect("tap stays live")
                .expect("tap value");
            if published.subject == format!("p.{pair}.state.sessions")
                && published.json()["sessions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["title"] == json!("Renamed heartbeat"))
            {
                saw_rename = true;
            }
        }

        // Publish failures (broker dead) are logged, not fatal.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        std::fs::remove_dir_all(&workspace_dir).ok();
    }

    #[tokio::test]
    async fn heartbeat_logs_publish_failures_and_keeps_ticking() {
        let _home = HomeGuard::new("remote-heartbeat-fail");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        // Payloads beyond the broker's max_payload cap are refused client-side:
        // an oversized pair id inflates the presence/sessions payloads and an
        // oversized workspace name inflates the workspaces payload, so every
        // heartbeat publish fails deterministically — no broker kill timing.
        let pair = "p".repeat(9 * 1024 * 1024);
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-hbf"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("n".repeat(9 * 1024 * 1024)),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();

        let handle = spawn_presence_heartbeat(client.clone(), pair, "bridge_hbf".into());
        // Several ticks run every publish arm; none of the failures is fatal.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!handle.is_finished());
        handle.abort();
        std::fs::remove_dir_all(&workspace_dir).ok();
    }

    #[tokio::test]
    async fn web_server_serves_files_and_rejects_bad_requests() {
        let _home = HomeGuard::new("remote-web");
        // Point the handler at a fixture dir (the real web dir only ships
        // index.html; the content-type arms need .js/.css/other files).
        let dir = std::env::temp_dir().join(unique("futureos-web"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>hi</html>").unwrap();
        std::fs::write(dir.join("app.js"), "console.log(1)").unwrap();
        std::fs::write(dir.join("style.css"), "body{}").unwrap();
        std::fs::write(dir.join("logo.dat"), vec![0_u8; 8]).unwrap();

        async fn request(dir: &std::path::Path, raw: &str) -> String {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let accept = tokio::spawn({
                let dir = dir.to_path_buf();
                async move {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    handle_web_request(&mut stream, &dir).await;
                }
            });
            let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            client.write_all(raw.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            accept.await.unwrap();
            String::from_utf8_lossy(&response).to_string()
        }

        let response = request(&dir, "GET / HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("200 OK"), "{response}");
        assert!(response.contains("text/html"), "{response}");
        assert!(response.contains("<html>hi</html>"), "{response}");

        let response = request(&dir, "GET /app.js HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("application/javascript"), "{response}");

        let response = request(&dir, "GET /style.css HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("text/css"), "{response}");

        let response = request(&dir, "GET /logo.dat HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("application/octet-stream"), "{response}");

        let response = request(&dir, "GET /missing.txt HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("404"), "{response}");

        let response = request(&dir, "GET /../secret HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("403"), "{response}");

        // A request line without a path defaults to the index.
        let response = request(&dir, "GARBAGE\r\n\r\n").await;
        assert!(response.contains("200 OK"), "{response}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn web_server_drops_silent_clients() {
        let _home = HomeGuard::new("remote-web-timeout");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(unique("futureos-webt"));
        std::fs::create_dir_all(&dir).unwrap();
        let accept = tokio::spawn({
            let dir = dir.clone();
            async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                handle_web_request(&mut stream, &dir).await;
            }
        });
        let client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        // Never send a request: the read times out and the handler returns.
        tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .expect("handler returns on read timeout")
            .unwrap();
        drop(client);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lan_ip_returns_an_ipv4_literal_or_none() {
        let _home = HomeGuard::new("remote-lan");
        // `lan_ip` may be None when the host has no routable probe address; when
        // it yields a literal it must be a valid IPv4 address.
        let ipv4 = lan_ip()
            .map(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
            .unwrap_or(true);
        assert!(ipv4, "lan_ip yielded a non-IPv4 literal");
    }

    #[test]
    fn presence_payloads_are_built_from_the_store() {
        let _home = HomeGuard::new("remote-snapshots");
        init_store();

        // Empty store → empty but well-formed snapshots.
        let (payload, signature) = build_presence_snapshot("pair_x", "bridge_x");
        assert_eq!(payload["online"], json!(true));
        assert_eq!(payload["sessions"], json!([]));
        assert_eq!(payload["workspaces"], json!([]));
        assert!(signature.is_empty());
        assert_eq!(payload, build_presence_payload("pair_x", "bridge_x"));

        let light = light_presence_payload("pair_x", "bridge_x");
        assert_eq!(light["online"], json!(true));
        assert!(light.get("sessions").is_none());

        // Threads without an agent session are skipped; workspaces are
        // filtered to user kind.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Snap".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let (payload, sig_without_session) =
            build_sessions_snapshot("pair_x").expect("store readable");
        assert_eq!(payload["sessions"], json!([]));
        // The full presence snapshot skips session-less threads the same way.
        let (payload, _) = build_presence_snapshot("pair_x", "bridge_x");
        assert_eq!(payload["sessions"], json!([]));
        crate::store::update_thread_session_id(&thread.id, "sess-snap").unwrap();
        let (payload, sig_with_session) =
            build_sessions_snapshot("pair_x").expect("store readable");
        assert_eq!(payload["sessions"].as_array().unwrap().len(), 1);
        assert_ne!(sig_without_session, sig_with_session);

        // …and once the thread has a session, the full snapshot carries it.
        let (payload, signature) = build_presence_snapshot("pair_x", "bridge_x");
        let row = payload["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["sessionId"] == json!("sess-snap"))
            .expect("presence snapshot includes the session thread");
        assert_eq!(row["threadId"], json!(thread.id));
        assert_eq!(row["streaming"], json!(false));
        assert!(!signature.is_empty());

        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws2"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Snap WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        let (payload, signature) = build_workspaces_snapshot();
        assert!(payload["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["name"] == json!("Snap WS")));
        assert!(!signature.is_empty());
        std::fs::remove_dir_all(&workspace_dir).ok();

        // The full presence snapshot carries the user-kind workspace too.
        let (payload, _) = build_presence_snapshot("pair_x", "bridge_x");
        assert!(payload["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["name"] == json!("Snap WS")));
    }

    #[test]
    fn cap_event_data_only_truncates_beyond_the_budget() {
        let _home = HomeGuard::new("remote-cap");
        let small = cap_event_data("small");
        assert!(matches!(small, std::borrow::Cow::Borrowed(_)));
        let big_text = "x".repeat(MAX_EVENT_BYTES + 1);
        let big = cap_event_data(&big_text);
        assert!(big.contains("_truncated"));
        assert!(big.contains("full content is available via get_messages"));
    }
}
