//! Live client connections, one per paired remote host.
//!
//! Scope, stated plainly because the omissions are deliberate: this holds
//! *connections* and answers requests. It is not yet the reconnecting
//! supervisor the host side has — no automatic retry, no credential-refresh
//! timer, no event fan-out into the UI. A caller that wants a connection opens
//! one ([`ensure_connected`]); a dead socket surfaces as an error and the next
//! request opens a fresh one. Everything that would make a *background*
//! promise (retry budgets, sleep/wake, refresh scheduling) is left out rather
//! than half-implemented, because a client that silently stops delivering
//! events is worse than one that is visibly disconnected.
//!
//! Sessions are keyed by the remote host's desktop id — the identity the user
//! sees and renames. The pair id is carried alongside but is not the key: a
//! re-pair of the same host replaces its entry rather than adding a second one,
//! which is the same rule the credential book enforces.

use super::creds::{self, PeerCreds};
use super::session::{self, PeerSession};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex;

/// What the UI shows for one paired host. Never carries a token or a seed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerSummary {
    pub desktop_id: String,
    pub pair_id: String,
    /// The user's name for this host, if they set one. The UI falls back to a
    /// short form of the id.
    pub name: Option<String>,
    /// The user's chosen glyph (see the icon picker). Rendered in place of the
    /// full name in the merged list, which is why it exists at all.
    pub icon: Option<String>,
    pub connected: bool,
    /// Last connection error for this host, cleared on a successful connect.
    pub error: Option<String>,
    /// The host's bridge instance while connected; `None` when not.
    pub bridge_instance_id: Option<String>,
    /// Capabilities the host declared in its handshake confirmation. The UI
    /// gates optional actions (file transfer, compaction, fork) on these
    /// instead of guessing from a version string.
    pub features: Vec<String>,
    /// The host's own statement, at handshake time, whether its *agent* was
    /// reachable. A reachable bridge with an unavailable agent is the case the
    /// support table calls `LC003`: the link is up and every command fails.
    pub agent_available: bool,
}

/// Where pushed events go. Injected rather than hard-coded to Tauri so the
/// runtime stays testable and the desktop app is not the only possible host.
pub(crate) type Emitter = std::sync::Arc<dyn Fn(PeerEvent) + Send + Sync>;

/// One decrypted push from a host, forwarded to the UI verbatim.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerEvent {
    pub desktop_id: String,
    /// `event` for `evt.*` (a session event, which carries its own `sessionId`),
    /// `presence` for a liveness tick.
    pub kind: &'static str,
    pub payload: Value,
}

#[derive(Default)]
struct Runtime {
    live: HashMap<String, PeerSession>,
    /// The subscription task per host, aborted on disconnect so a host the user
    /// removed cannot keep emitting into a UI that has forgotten it.
    tasks: HashMap<String, tokio::task::JoinHandle<()>>,
    /// The reconnect task per host, aborted on an explicit disconnect so a
    /// deliberate stop cannot be undone by a retry that was already scheduled.
    supervisors: HashMap<String, tokio::task::JoinHandle<()>>,
    /// The last failure per host. Kept after a disconnect so the list can say
    /// *why* a host is not connected instead of only that it is not.
    errors: HashMap<String, String>,
}

/// The retry schedule for a dropped connection, in seconds.
///
/// Mirrors the contract the phone implements: bounded backoff with jitter, and
/// a total window after which the client stops trying and reports a terminal
/// failure. An unbounded retry against a host that will never come back is
/// indistinguishable, to the user, from a hung app.
const RECONNECT_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

/// The unit the schedule's numbers are counted in.
///
/// Seconds in production; milliseconds under test, so the suite does not spend
/// real minutes waiting out backoff. The *shape* being exercised is the real
/// one either way — scaling the unit is what keeps the schedule itself honest.
#[cfg(not(test))]
fn delay_unit() -> std::time::Duration {
    std::time::Duration::from_secs(1)
}
#[cfg(test)]
fn delay_unit() -> std::time::Duration {
    std::time::Duration::from_millis(1)
}

/// The un-jittered delay for an attempt, in seconds.
fn reconnect_base_secs(attempt: usize) -> u64 {
    RECONNECT_DELAYS_SECS[attempt.min(RECONNECT_DELAYS_SECS.len() - 1)]
}

fn reconnect_delay(attempt: usize) -> std::time::Duration {
    let base = reconnect_base_secs(attempt) as f64;
    // ±20% jitter: several hosts coming back after a network blip must not
    // stampede the relay in the same second.
    let jitter = 1.0 + (rand::random::<f64>() - 0.5) * 0.4;
    delay_unit().mul_f64(base * jitter)
}

fn runtime() -> &'static Mutex<Runtime> {
    static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(Runtime::default()))
}

/// Drop every live connection and background task.
///
/// The runtime is a process-global singleton, so a test that inherits the
/// previous one's `live` map would be asserting against another test's socket.
/// `HomeGuard` already serializes the tests and cancels the tasks that outlived
/// their owner; this clears the state those tasks left behind.
#[cfg(test)]
pub(crate) async fn reset_for_test() {
    let mut live = runtime().lock().await;
    for (_, task) in live.tasks.drain() {
        task.abort();
    }
    for (_, supervisor) in live.supervisors.drain() {
        supervisor.abort();
    }
    live.live.clear();
    live.errors.clear();
}

/// How many hosts are currently connected. Tests assert on counts rather than
/// poking at the map, so the test does not have to hold the runtime lock while
/// it reasons about the result.
#[cfg(test)]
pub(crate) async fn live_count() -> usize {
    runtime().lock().await.live.len()
}

/// How many event-stream tasks are *running*.
///
/// Finished handles stay in the map until something replaces them, so counting
/// the map would report a task that has already returned.
#[cfg(test)]
pub(crate) async fn stream_count() -> usize {
    runtime()
        .lock()
        .await
        .tasks
        .values()
        .filter(|handle| !handle.is_finished())
        .count()
}

/// How many retry loops are running.
#[cfg(test)]
pub(crate) async fn supervisor_count() -> usize {
    runtime().lock().await.supervisors.len()
}

/// Close a live connection's socket without touching the retry state.
///
/// The state a process shutting down leaves behind: the subscription streams
/// end, so a test can drive the stream task's own exit instead of racing a
/// broker shutdown (a reconnecting client keeps its subscriptions open).
#[cfg(test)]
pub(crate) async fn close_socket_for_test(desktop_id: &str) {
    let session = runtime()
        .lock()
        .await
        .live
        .remove(desktop_id)
        .expect("a live session for the test to close");
    let _ = session.close_socket().await;
}
///
/// Pretend a retry loop is already running for this host.
///
/// The state a transport failure leaves behind, so a test can drive "a connect
/// found a retry loop and replaced it" without first having to induce the
/// failure and wait for the loop to be installed.
#[cfg(test)]
pub(crate) async fn install_supervisor_for_test(desktop_id: &str) {
    let handle = spawn_supervisor(desktop_id.to_string(), None);
    runtime()
        .lock()
        .await
        .supervisors
        .insert(desktop_id.to_string(), handle);
}

/// Renew the grant and persist it.
///
/// Persisting is part of the operation, not a nicety: a rotated token that is
/// only held in memory makes the *next* launch renew again from the token this
/// process already replaced.
async fn renew_credentials(creds: &PeerCreds) -> Result<PeerCreds, crate::AppError> {
    let fresh = super::platform::refresh(creds).await?;
    creds::update_credentials(
        &creds.desktop_id,
        fresh.user_jwt.clone(),
        fresh.nats_url.clone(),
        fresh.nats_ws_url.clone(),
        fresh.jwt_expires_at,
    )?;
    let mut renewed = creds.clone();
    renewed.user_jwt = fresh.user_jwt;
    renewed.nats_url = fresh.nats_url;
    renewed.nats_ws_url = fresh.nats_ws_url;
    renewed.jwt_expires_at = fresh.jwt_expires_at;
    Ok(renewed)
}

/// Forget a connection without the command path noticing — the state a socket
/// drop leaves behind, so a test can drive the recovery path deterministically
/// instead of racing a broker shutdown.
#[cfg(test)]
pub(crate) async fn forget_connection_for_test(desktop_id: &str) {
    runtime().lock().await.live.remove(desktop_id);
}

/// Pair with a remote host from a pasted `futureos://remote/pair` link, then
/// connect once so the caller learns immediately whether it worked.
///
/// The link failures carry the mobile client's `PA*` support codes, so both
/// platforms name the same fault the same way and a support conversation means
/// the same thing on either.
pub(crate) async fn pair_with_emitter(
    invitation: &str,
    emitter: Option<Emitter>,
) -> Result<PeerSummary, crate::AppError> {
    let parsed = super::link::parse_invitation(invitation, now_secs()).map_err(|error| {
        crate::AppError::Remote {
            status: 0,
            code: Some(error.support_code().to_string()),
            message: error.message().to_string(),
        }
    })?;
    let device = nkeys::KeyPair::new_user();
    let (private, public) = future_remote_crypto::generate_identity()
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let device_id = crate::device_identity::device_id()?;
    let claimed =
        super::platform::claim(&parsed, &device_id, &device.public_key(), &device_name()).await?;
    let creds = PeerCreds {
        pair_id: claimed.pair_id,
        desktop_id: parsed.desktop_id.clone(),
        device_id,
        nkey_seed: device
            .seed()
            .map_err(|error| crate::AppError::Message(format!("generate device NKey: {error}")))?,
        user_jwt: claimed.user_jwt,
        refresh_token: claimed.refresh_token,
        nats_url: claimed.nats_url,
        nats_ws_url: claimed.nats_ws_url,
        jwt_expires_at: claimed.jwt_expires_at,
        token_url: parsed.claim_url.replace("/pair/claim", "/auth/token"),
        secure: Some(creds::PeerIdentity {
            private_key: URL_SAFE_NO_PAD.encode(private),
            public_key: URL_SAFE_NO_PAD.encode(&public),
            // Known from the link, not learned from the handshake: the first
            // connection is authenticated by the invitation itself.
            peer_public_key: Some(parsed.secure_key.clone()),
            secret: Some(parsed.secret.clone()),
        }),
    };
    let desktop_id = creds.desktop_id.clone();
    creds::upsert(creds)?;
    connect_with_emitter(&desktop_id, emitter).await
}

/// What the remote host shows in its "a new device paired" state. The platform
/// stores it verbatim, so it is the only place the host can learn that this
/// machine (rather than a phone) took the slot.
fn device_name() -> String {
    device_name_from(
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .ok(),
    )
}

/// The formatting half, split out so both arms are testable without mutating
/// the process environment (which no test can do safely alongside others).
fn device_name_from(host: Option<String>) -> String {
    let host = host
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    match host {
        Some(host) => format!("FutureOS Desktop ({host})"),
        None => "FutureOS Desktop".to_string(),
    }
}

/// The book's view of one host, without any live state.
fn paired_summary(desktop_id: &str) -> Result<PeerSummary, crate::AppError> {
    let book = creds::load()?;
    let peer = book
        .find(desktop_id)
        .ok_or_else(|| crate::AppError::Message("peer_not_paired".into()))?;
    Ok(PeerSummary {
        desktop_id: peer.creds.desktop_id.clone(),
        pair_id: peer.creds.pair_id.clone(),
        name: peer.label.name.clone(),
        icon: peer.label.icon.clone(),
        connected: false,
        error: None,
        bridge_instance_id: None,
        features: Vec::new(),
        // Not connected yet, so nothing has been heard from the host. `false`
        // rather than `true` keeps "unknown" on the safe side of the UI's
        // "everything is fine" check.
        agent_available: false,
    })
}

/// Every paired host, whether or not it is currently connected.
pub(crate) async fn list() -> Result<Vec<PeerSummary>, crate::AppError> {
    let book = creds::load()?;
    let live = runtime().lock().await;
    Ok(book
        .peers
        .iter()
        .map(|peer| {
            let session = live.live.get(&peer.creds.desktop_id);
            PeerSummary {
                desktop_id: peer.creds.desktop_id.clone(),
                pair_id: peer.creds.pair_id.clone(),
                name: peer.label.name.clone(),
                icon: peer.label.icon.clone(),
                connected: session.is_some(),
                error: live.errors.get(&peer.creds.desktop_id).cloned(),
                bridge_instance_id: session.map(|session| session.bridge_instance_id().to_string()),
                features: session
                    .map(|session| session.features().to_vec())
                    .unwrap_or_default(),
                agent_available: session.is_some_and(|session| session.agent_available()),
            }
        })
        .collect())
}

/// Open a connection unless one is already live.
pub(crate) async fn ensure_connected(desktop_id: &str) -> Result<PeerSummary, crate::AppError> {
    if let Some(peer) = list()
        .await?
        .into_iter()
        .find(|peer| peer.desktop_id == desktop_id)
        .filter(|peer| peer.connected)
    {
        return Ok(peer);
    }
    connect(desktop_id).await
}

/// Connect (replacing any existing connection for this host). A failed attempt
/// is recorded against the host so the list can explain it, then returned.
pub(crate) async fn connect(desktop_id: &str) -> Result<PeerSummary, crate::AppError> {
    connect_with_emitter(desktop_id, None).await
}

/// Connect and, when an emitter is supplied, start streaming that host's events
/// into it.
pub(crate) async fn connect_with_emitter(
    desktop_id: &str,
    emitter: Option<Emitter>,
) -> Result<PeerSummary, crate::AppError> {
    let mut creds = stored_creds(desktop_id)?;
    // Renew before connecting: a token that expires mid-handshake fails it in a
    // way that looks like a protocol fault.
    if creds.needs_refresh(now_secs()) {
        match renew_credentials(&creds).await {
            Ok(renewed) => creds = renewed,
            Err(error) => {
                // A refused or unpersistable renewal happens before any socket
                // exists, so it must be recorded here — otherwise the UI shows a
                // host that is simply "not connected" with no reason, while
                // nothing will ever retry it.
                runtime()
                    .lock()
                    .await
                    .errors
                    .insert(desktop_id.to_string(), error.to_string());
                return Err(error);
            }
        }
    }
    let connected = match session::connect(&creds).await {
        Ok(connected) => connected,
        Err(error) => {
            runtime()
                .lock()
                .await
                .errors
                .insert(desktop_id.to_string(), error.to_string());
            return Err(error);
        }
    };
    // The PSK has now been spent. Drop it from disk while we know the host has
    // bound this identity; from here every connection is `Noise_IK`.
    if connected.consumed_invitation {
        if let Err(error) = creds::clear_secret(desktop_id) {
            eprintln!("remote_peer: could not clear the invitation secret: {error}");
        }
    }
    let mut summary = paired_summary(desktop_id)?;
    summary.connected = true;
    // Subscribe before taking the runtime lock: subscription is a network
    // round-trip, and a failure there must not keep a usable connection out of
    // the map.
    let peer_session = connected.session;
    let stream = match emitter {
        Some(emitter) => attach_stream(&peer_session, emitter, desktop_id).await,
        None => None,
    };
    {
        let mut live = runtime().lock().await;
        live.errors.remove(desktop_id);
        summary.bridge_instance_id = Some(peer_session.bridge_instance_id().to_string());
        summary.features = peer_session.features().to_vec();
        summary.agent_available = peer_session.agent_available();
        // A reconnect replaces the stream: the old task's socket is gone, and
        // leaving it running would emit from a dead connection.
        if let Some(previous) = live.tasks.remove(desktop_id) {
            previous.abort();
        }
        if let Some((channel, events, presence, emitter)) = stream {
            live.tasks.insert(
                desktop_id.to_string(),
                spawn_event_stream(desktop_id.to_string(), channel, events, presence, emitter),
            );
        }
        live.live.insert(desktop_id.to_string(), peer_session);
        // A live connection is proof the retry loop is no longer needed; it is
        // restarted below only if this connection later drops.
        if let Some(supervisor) = live.supervisors.remove(desktop_id) {
            supervisor.abort();
        }
    }
    Ok(summary)
}

/// How many attempts the supervisor makes before it stops for good.
///
/// The schedule's length times three: the window is what makes a failure
/// terminal, and a client that retries forever looks identical, to the user, to
/// one that hung.
fn reconnect_attempt_budget() -> usize {
    RECONNECT_DELAYS_SECS.len() * 3
}
///
/// Keep one host connected: watch the socket, and reconnect when it drops.
///
/// Runs until it succeeds, until its retry window is exhausted, or until it is
/// aborted (an explicit disconnect, or a newer connection replacing it).
pub(crate) fn spawn_supervisor(
    desktop_id: String,
    emitter: Option<Emitter>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut attempt = 0usize;
        loop {
            tokio::time::sleep(reconnect_delay(attempt)).await;
            // Already connected (something else reconnected while we waited).
            let connected = runtime().lock().await.live.contains_key(&desktop_id);
            if connected {
                return;
            }
            match resume(&desktop_id, emitter.clone()).await {
                Ok(()) => return,
                Err(error) => {
                    eprintln!("remote_peer: reconnect attempt for {desktop_id} failed: {error}");
                }
            }
            attempt += 1;
            // Measured in *attempts*, not wall clock: the delay schedule already
            // caps the wait, and a clock here would need a monotonic source the
            // tests would have to fake.
            if attempt >= reconnect_attempt_budget() {
                return;
            }
        }
    })
}

/// Refresh credentials when needed, then reconnect and re-subscribe.
async fn resume(desktop_id: &str, emitter: Option<Emitter>) -> Result<(), crate::AppError> {
    connect_with_emitter(desktop_id, emitter).await.map(|_| ())
}

/// The stream half of a connection: the traffic-key handle and the two
/// subscriptions the event task drains.
type StreamParts = (
    std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    async_nats::Subscriber,
    async_nats::Subscriber,
    Emitter,
);

/// Subscribe to a host's pushes, or say why not.
///
/// A failure here leaves the connection *usable for commands* — only the live
/// half is missing, and the UI falls back to fetching history — so it is
/// reported and turned into `None` rather than propagated. Split out from the
/// connect path so this decision is reachable without a broker that accepts a
/// handshake and then refuses a subscription.
async fn attach_stream(
    peer_session: &PeerSession,
    emitter: Emitter,
    desktop_id: &str,
) -> Option<StreamParts> {
    match peer_session.subscribe().await {
        Ok((events, presence)) => {
            Some((peer_session.channel_for_stream(), events, presence, emitter))
        }
        Err(error) => {
            eprintln!("remote_peer: could not subscribe for {desktop_id}: {error}");
            None
        }
    }
}

/// Drain the host's event and presence subscriptions into `emitter`.
///
/// The traffic-key mutex is shared with the command path, so this task and a
/// concurrent request take the same short lock; neither holds it across an
/// `await`.
fn spawn_event_stream(
    desktop_id: String,
    channel: std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    mut events: async_nats::Subscriber,
    mut presence: async_nats::Subscriber,
    emitter: Emitter,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        fn open(
            channel: &std::sync::Mutex<future_remote_crypto::Channel>,
            subject: &str,
            payload: &[u8],
        ) -> Option<Value> {
            let opened = channel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .open(subject, payload)
                .ok()?;
            serde_json::from_slice(&opened).ok()
        }
        loop {
            let next = tokio::select! {
                message = events.next() => message.map(|message| ("event", message)),
                message = presence.next() => message.map(|message| ("presence", message)),
            };
            let Some((kind, message)) = next else {
                // Both streams ended: the socket is gone. The next status poll
                // and the next command both notice; this task just stops.
                return;
            };
            let Some(payload) = open(&channel, &message.subject, &message.payload) else {
                // Unauthenticated or unparsable: a relay can inject either at
                // will and the connection is not at fault, so drop it.
                continue;
            };
            emitter(PeerEvent {
                desktop_id: desktop_id.clone(),
                kind,
                payload,
            });
        }
    })
}

/// Drop the connection but keep the pairing (the user's "disconnect").
pub(crate) async fn disconnect(desktop_id: &str) {
    let mut live = runtime().lock().await;
    if let Some(task) = live.tasks.remove(desktop_id) {
        task.abort();
    }
    // Without this, a retry scheduled a second ago would silently bring the
    // connection back after the user asked for it to stop.
    if let Some(supervisor) = live.supervisors.remove(desktop_id) {
        supervisor.abort();
    }
    live.live.remove(desktop_id);
}

/// Drop the pairing locally, then try to revoke it server-side.
///
/// Local first, always: an unreachable platform must not leave a host the user
/// removed still connectable. A revoke failure is *returned* rather than
/// swallowed so a caller can queue a retry, but it is not fatal — pairing again
/// is a new invitation either way.
pub(crate) async fn unpair(desktop_id: &str) -> Result<Option<String>, crate::AppError> {
    let creds = stored_creds(desktop_id)?;
    disconnect(desktop_id).await;
    creds::remove(desktop_id)?;
    Ok(super::platform::revoke(&creds)
        .await
        .err()
        .map(|error| error.to_string()))
}

pub(crate) fn set_label(
    desktop_id: &str,
    name: Option<&str>,
    icon: Option<&str>,
) -> Result<(), crate::AppError> {
    creds::set_label(desktop_id, name, icon)
}

/// The remote catalogue, stamped with the host it came from.
///
/// The stamp is added here rather than derived by the caller: a merged list is
/// built from several hosts' snapshots, and a row that cannot name its host
/// would be routed to the wrong machine on click.
pub(crate) async fn sessions(desktop_id: &str) -> Result<Value, crate::AppError> {
    let data = request(desktop_id, json!({ "type": "list_sessions" }), "list").await?;
    stamp_live(desktop_id, data).await
}

pub(crate) async fn workspaces(desktop_id: &str) -> Result<Value, crate::AppError> {
    let data = request(desktop_id, json!({ "type": "list_workspaces" }), "list").await?;
    stamp_live(desktop_id, data).await
}

/// One command to one host.
///
/// `lane` is the fallback routing token; when the command carries a
/// `sessionId`, that is what the host routes on, so a caller cannot address a
/// conversation on a host it did not name.
pub(crate) async fn request(
    desktop_id: &str,
    mut command: Value,
    lane: &str,
) -> Result<Value, crate::AppError> {
    ensure_connected(desktop_id).await?;
    if let Some(object) = command.as_object_mut() {
        object
            .entry("id")
            .or_insert_with(|| json!(crate::store::create_id("cmd")));
    }
    let key = if lane == "list" {
        "list".to_string()
    } else {
        command
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or(lane)
            .to_string()
    };
    let mut live = runtime().lock().await;
    let Some(session) = live.live.get_mut(desktop_id) else {
        return Err(crate::AppError::Message("peer_not_connected".into()));
    };
    let subject = session.command_subject(&key);
    let result = session
        .request(&subject, command, session::COMMAND_TIMEOUT)
        .await;
    if let Err(error) = &result {
        // A dead socket must not keep looking connected on the next poll.
        if matches!(error, crate::AppError::RemoteTransport(_)) {
            live.errors
                .insert(desktop_id.to_string(), error.to_string());
            live.live.remove(desktop_id);
            // The connection was healthy and then was not: exactly the case
            // automatic recovery is for. An explicit disconnect takes the
            // supervisor with it, so this cannot resurrect a stopped link.
            if !live.supervisors.contains_key(desktop_id) {
                let supervisor = spawn_supervisor(desktop_id.to_string(), None);
                live.supervisors.insert(desktop_id.to_string(), supervisor);
            }
        }
    }
    result
}

/// Stamp a catalogue snapshot with the host and the *live* pairing it came
/// from.
///
/// Both ids travel: `desktopId` is what the UI keys a row by, and `pairId` is
/// what the host used. They diverge after a re-pair of the same machine, so a
/// row carrying only one of them could be routed to a connection the other no
/// longer matches. The pair id is read from the live connection rather than the
/// credential file, so a stale file cannot label a row with a pairing the
/// socket is not speaking.
async fn stamp_live(desktop_id: &str, data: Value) -> Result<Value, crate::AppError> {
    let pair_id = runtime()
        .lock()
        .await
        .live
        .get(desktop_id)
        .map(|session| session.pair_id().to_string());
    let mut data = data;
    if let Some(object) = data.as_object_mut() {
        object.insert("desktopId".into(), json!(desktop_id));
        if let Some(pair_id) = pair_id {
            object.insert("pairId".into(), json!(pair_id));
        }
    }
    Ok(data)
}

fn stored_creds(desktop_id: &str) -> Result<PeerCreds, crate::AppError> {
    creds::load()?
        .find(desktop_id)
        .map(|peer| peer.creds.clone())
        .ok_or_else(|| crate::AppError::Message("peer_not_paired".into()))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_peer::testing::{fixture, teardown};

    /// An emitter that records nothing; these tests are about whether the stream
    /// attaches, not about what it carries.
    fn silent_emitter() -> Emitter {
        std::sync::Arc::new(|_event: PeerEvent| {})
    }

    /// Take a session out of the runtime so a test owns it (a `SecureChannel` is
    /// not `Clone`, and taking it is also what removes it from the serving path).
    async fn take_session(desktop_id: &str) -> PeerSession {
        runtime()
            .lock()
            .await
            .live
            .remove(desktop_id)
            .expect("a live session")
    }

    /// The runtime is a process-global singleton, so each of these starts from a
    /// clean one, and the fixture is returned so the host bridge and its broker
    /// outlive the call — dropping it would stop the server under test.
    async fn connected_fixture(
        label: &str,
    ) -> (
        crate::remote::test_support::HomeGuard,
        crate::remote_peer::testing::Fixture,
        String,
    ) {
        reset_for_test().await;
        let (home, fx) = fixture(label).await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        connect(&desktop_id).await.expect("connect");
        (home, fx, desktop_id)
    }

    /// A control for the test below: on a healthy socket the stream attaches, so
    /// that test is about the closed socket rather than about `attach_stream`
    /// refusing everything.
    #[tokio::test]
    async fn a_fresh_socket_attaches_its_stream() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-attach-fresh").await;
        let session = take_session(&desktop_id).await;
        assert!(
            attach_stream(&session, silent_emitter(), &desktop_id)
                .await
                .is_some(),
            "a healthy socket must attach"
        );
        drop(fx);
        teardown().await;
    }

    /// A subscription cannot be installed on a socket that is gone, and the
    /// connection must stay usable for commands when that happens: only the live
    /// half is missing, and the UI falls back to fetching history.
    #[tokio::test]
    async fn attaching_a_stream_to_a_closed_socket_reports_no_stream() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-attach-closed").await;
        let session = take_session(&desktop_id).await;
        session.close_socket().await.expect("close the socket");
        // Let the close take effect before the subscribe is attempted.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            attach_stream(&session, silent_emitter(), &desktop_id)
                .await
                .is_none(),
            "a closed socket cannot carry a subscription"
        );
        drop(fx);
        teardown().await;
    }

    /// A stored seed that cannot be read is a local credential fault, and must be
    /// reported as one rather than sent to the host as an empty key.
    #[tokio::test]
    async fn a_stored_seed_that_cannot_be_read_is_reported() {
        let (_home, fx, _desktop_id) = connected_fixture("peer-rt-bad-seed").await;
        let mut broken = fx.paired.creds.clone();
        broken.nkey_seed = "not-a-seed".into();
        // Matching rather than `expect_err`: the ok arm holds a live connection,
        // which is neither `Debug` nor something to print on failure.
        let error = match super::super::session::connect(&broken).await {
            Ok(_) => panic!("an invalid seed cannot sign"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("Invalid stored device NKey"),
            "{error}"
        );
        drop(fx);
        teardown().await;
    }

    /// A handshake sent on a socket that has gone cannot be answered, and the
    /// failure has to reach the caller as a transport fault rather than a hang.
    #[tokio::test]
    async fn a_handshake_on_a_closed_socket_is_a_transport_failure() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-handshake-closed").await;
        let session = take_session(&desktop_id).await;
        session.close_socket().await.expect("close the socket");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let error = super::super::session::exchange_for_test(
            session.client_for_test(),
            "p.pair_1.cmd.handshake",
            serde_json::json!({ "type": "secure_open" }),
        )
        .await
        .expect_err("a closed socket cannot carry a handshake");
        assert!(
            matches!(error, crate::AppError::RemoteTransport(_)),
            "{error}"
        );
        drop(fx);
        teardown().await;
    }

    /// The *base* schedule is what must grow and be bounded; jitter only
    /// perturbs each draw around its step, so a jittered sample is deliberately
    /// not monotonic (30s×1.2 can exceed the next step's 30s×0.8 — that is the
    /// point of jitter). Asserting on the drawn values would test the noise.
    #[test]
    fn the_reconnect_schedule_grows_and_is_capped() {
        let bases: Vec<u64> = (0..RECONNECT_DELAYS_SECS.len() + 5)
            .map(reconnect_base_secs)
            .collect();
        for pair in bases.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "the schedule must not shrink: {bases:?}"
            );
        }
        assert_eq!(
            RECONNECT_DELAYS_SECS.last(),
            Some(&30),
            "the last step is the cap"
        );
        // Past the end the delay stays at the cap rather than growing without
        // bound, and every draw stays inside the jitter band for its step.
        let unit = delay_unit().as_secs_f64();
        for attempt in 0..RECONNECT_DELAYS_SECS.len() + 5 {
            let base = reconnect_base_secs(attempt) as f64;
            for _ in 0..16 {
                let ratio = reconnect_delay(attempt).as_secs_f64() / unit;
                assert!(
                    ratio >= base * 0.8 && ratio <= base * 1.2,
                    "attempt {attempt}: {ratio} is outside the ±20% band around {base}"
                );
            }
        }
    }

    /// Jitter is what keeps several hosts from retrying in the same instant, so
    /// it has to actually vary — a constant would pass the range check above.
    #[test]
    fn reconnect_delays_are_jittered_not_constant() {
        let unit = delay_unit().as_secs_f64();
        let samples: Vec<f64> = (0..64)
            .map(|_| reconnect_delay(5).as_secs_f64() / unit)
            .collect();
        let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(max - min > 0.5, "delay never varied: {min}..{max}");
    }

    /// The window is finite, and it is a multiple of the schedule: a client that
    /// retries forever is indistinguishable from one that hung.
    #[test]
    fn the_attempt_budget_is_bounded() {
        assert_eq!(reconnect_attempt_budget(), RECONNECT_DELAYS_SECS.len() * 3);
        assert!(reconnect_attempt_budget() > 0);
    }

    #[test]
    fn the_hostname_is_used_when_present_and_omitted_when_not() {
        assert_eq!(
            device_name_from(Some("studio-imac".into())),
            "FutureOS Desktop (studio-imac)"
        );
        // Whitespace is not a hostname; an empty name would produce a trailing
        // "()" that reads like a bug in the host's UI.
        assert_eq!(device_name_from(Some("   ".into())), "FutureOS Desktop");
        assert_eq!(
            device_name_from(Some("  padded  ".into())),
            "FutureOS Desktop (padded)"
        );
        assert_eq!(device_name_from(None), "FutureOS Desktop");
    }

    /// The environment-reading wrapper still resolves to a usable name. It is
    /// asserted as an *invariant* rather than a specific value, because the host
    /// it produces depends on the machine running the test.
    #[test]
    fn the_live_hostname_resolves_to_a_usable_name() {
        let name = device_name();
        assert!(name.starts_with("FutureOS Desktop"), "{name}");
        assert!(!name.ends_with("()"), "{name}");
    }

    /// An explicit disconnect must take the retry with it, or a reconnect
    /// scheduled a moment earlier silently undoes the user's decision.
    #[tokio::test]
    async fn disconnect_aborts_the_supervisor() {
        let _home = crate::remote::test_support::HomeGuard::new("peer-disconnect-supervisor");
        reset_for_test().await;
        let handle = spawn_supervisor("desktop_a".into(), None);
        runtime()
            .lock()
            .await
            .supervisors
            .insert("desktop_a".into(), handle);
        disconnect("desktop_a").await;
        assert!(
            !runtime().lock().await.supervisors.contains_key("desktop_a"),
            "the supervisor must not outlive an explicit disconnect"
        );
    }
}
