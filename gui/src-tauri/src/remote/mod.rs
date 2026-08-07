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
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

/// Port for the embedded web client HTTP server.
const WEB_PORT: u16 = 8022;

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
    /// Desktop identity bound into the QR invitation and signed handshake.
    pub desktop_id: String,
    pub desktop_public_key: String,
    /// Web client URL for this machine (localhost); `None` if the web server
    /// failed to bind.
    pub web_url: Option<String>,
    /// Web client URL a phone on the same LAN can reach; `None` if unavailable.
    pub web_lan_url: Option<String>,
    /// Machine-readable reason the bridge isn't healthy (e.g. `network`,
    /// `revoked`, `server`, `loop_dead`, `web_bind`). The UI localizes this via
    /// `error.<code>`; it is the preferred signal over [`Self::error`].
    pub error_code: Option<String>,
    /// Human-readable error text, used only when [`Self::error_code`] is `None`
    /// (an uncategorized local failure). When a code is present the UI shows
    /// the localized message and ignores this field.
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
        desktop_id: String::new(),
        desktop_public_key: String::new(),
        web_url: None,
        web_lan_url: None,
        error_code: None,
        error: None,
    }
}

pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    let _start_guard = START_LOCK.lock().await;
    let _ = stop();
    *LAST_ERROR_CODE.lock().unwrap() = None;

    // A remote/server failure here (offline, revoked, HTTP error) is not a
    // program fault — surface it as a localized, not-running status instead of
    // throwing a raw transport string at the UI. Local failures (NKey, disk)
    // keep propagating as `Err`.
    let (creds, pairing_code, pairing_code_expires_at) = match establish().await {
        Ok(value) => value,
        Err(error) => return start_failure(error),
    };
    let client = match connect_nats(&creds).await {
        Ok(client) => client,
        Err(error) => return start_failure(error),
    };
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
    // A failed web bind is non-fatal (the bridge still runs) but the UI should
    // explain why there's no local web client to point at.
    let web_bind_failed = web_task.is_none();

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: creds.nats_url.clone(),
        pair_id: pair_id.clone(),
        pairing_code: pairing_code.clone(),
        pairing_code_expires_at,
        desktop_id: creds.desktop_id.clone(),
        desktop_public_key: desktop_public_key.clone(),
        web_url: web_url.clone(),
        web_lan_url: web_lan_url.clone(),
        error_code: web_bind_failed.then(|| "web_bind".to_string()),
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
        heartbeat_task,
        refresh_task,
        web_task,
        web_url,
        web_lan_url,
        pairing_code,
        pairing_code_expires_at,
        pairing_confirmed,
    });
    Ok(status)
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
                "bridgeInstanceId": state.bridge_instance_id.clone(),
                "lastHeartbeatTs": unix_timestamp(),
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
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                pairing_code,
                pairing_code_expires_at,
                desktop_id: s.desktop_id.clone(),
                desktop_public_key: s.desktop_public_key.clone(),
                web_url: s.web_url.clone(),
                web_lan_url: s.web_lan_url.clone(),
                error_code: if loop_dead {
                    Some("loop_dead".to_string())
                } else if s.web_task.is_none() {
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
        None => RemoteStatus {
            error_code: LAST_ERROR_CODE.lock().unwrap().clone(),
            pair_id: pairing::load_creds().map(|c| c.pair_id).unwrap_or_default(),
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
    let Ok(payload) = serde_json::to_vec(&body) else {
        return;
    };
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
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut last_sessions_sig = String::new();
        let mut last_workspaces_sig = String::new();
        let mut secs_since_sessions: u8 = 20; // first tick publishes a baseline
        let mut secs_since_workspaces: u8 = 20;
        loop {
            interval.tick().await;

            // 1. Liveness micro-packet (every tick).
            if let Ok(bytes) =
                serde_json::to_vec(&light_presence_payload(&pair_id, &bridge_instance_id))
            {
                if let Err(e) = client
                    .publish(format!("p.{pair_id}.presence"), bytes.into())
                    .await
                {
                    eprintln!("remote: presence heartbeat write failed: {e}");
                }
            }

            // 2. Sessions snapshot (signature change or 20s self-heal).
            let dirty = crate::store::take_catalog_dirty();
            secs_since_sessions += 1;
            secs_since_workspaces += 1;
            let (sessions_payload, sessions_sig) = build_sessions_snapshot(&pair_id);
            if sessions_sig != last_sessions_sig || secs_since_sessions >= 20 {
                if let Ok(bytes) = serde_json::to_vec(&sessions_payload) {
                    if let Err(e) = client
                        .publish(format!("p.{pair_id}.state.sessions"), bytes.into())
                        .await
                    {
                        eprintln!("remote: state.sessions publish failed: {e}");
                    }
                }
                last_sessions_sig = sessions_sig;
                secs_since_sessions = 0;
            }

            // 3. Workspaces snapshot (dirty flag or 20s self-heal).
            let (workspaces_payload, workspaces_sig) = build_workspaces_snapshot();
            if dirty || workspaces_sig != last_workspaces_sig || secs_since_workspaces >= 20 {
                if let Ok(bytes) = serde_json::to_vec(&workspaces_payload) {
                    if let Err(e) = client
                        .publish(format!("p.{pair_id}.state.workspaces"), bytes.into())
                        .await
                    {
                        eprintln!("remote: state.workspaces publish failed: {e}");
                    }
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
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
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
                    *LAST_ERROR_CODE.lock().unwrap() = Some("revoked".to_string());
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
            let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
            let new_event = spawn_event_publisher(client.clone(), event_rx);
            let new_cmd = tokio::spawn(commands::command_loop(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
                handshake_state.clone(),
            ));
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
            "streaming": streaming,
            "status": status,
        }));
        signature.push('s');
        push_sig_field(&mut signature, sid);
        push_sig_field(&mut signature, &t.id);
        push_sig_field(&mut signature, &t.title);
        push_sig_field(&mut signature, &t.mode);
        push_sig_field(&mut signature, &t.workspace_id);
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
fn build_sessions_snapshot(pair_id: &str) -> (serde_json::Value, String) {
    let active_sessions: Vec<String> = crate::store::active_run_sessions().unwrap_or_default();
    let threads = crate::store::list_threads().unwrap_or_default();

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
            "streaming": streaming,
            "status": status,
        }));
        push_sig_field(&mut signature, sid);
        push_sig_field(&mut signature, &t.id);
        push_sig_field(&mut signature, &t.title);
        push_sig_field(&mut signature, &t.mode);
        push_sig_field(&mut signature, &t.workspace_id);
        push_sig_field(&mut signature, if streaming { "1" } else { "0" });
        push_sig_field(&mut signature, status.unwrap_or(""));
    }

    let payload = json!({
        "pairId": pair_id,
        "sessions": sessions,
    });
    (payload, signature)
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
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
