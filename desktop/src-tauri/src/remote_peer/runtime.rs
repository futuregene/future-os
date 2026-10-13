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
use std::time::Duration;
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
    /// `event` for a session event — one pushed on a conversation's `evt.`
    /// subject, or the same event read off the host-wide `state.events` mirror
    /// while the lane is narrowed to one conversation — `presence` for a
    /// liveness tick, `resumed` for a link that came back, `catalog` for a
    /// sessions or workspaces snapshot.
    ///
    /// `resumed` exists because the other kinds cannot express it: a presence tick
    /// arrives on a healthy link as well, and a session event says nothing about
    /// the link. Without a distinct signal, a client cannot tell "the host is
    /// fine" from "the host is back, and everything sent while it was gone is
    /// missing" — which is the whole reason the gap matters.
    pub kind: &'static str,
    pub payload: Value,
}

#[derive(Default)]
struct Runtime {
    live: HashMap<String, PeerSession>,
    /// The subscription task per host, aborted on disconnect so a host the user
    /// removed cannot keep emitting into a UI that has forgotten it.
    tasks: HashMap<String, crate::runtime::TaskHandle>,
    /// The reconnect task per host, aborted on an explicit disconnect so a
    /// deliberate stop cannot be undone by a retry that was already scheduled.
    supervisors: HashMap<String, crate::runtime::TaskHandle>,
    /// The last failure per host. Kept after a disconnect so the list can say
    /// *why* a host is not connected instead of only that it is not.
    errors: HashMap<String, String>,
    /// One inbox per chunk pull that is in flight, keyed by
    /// `(desktop, transfer, index)`.
    ///
    /// The bytes arrive on a *different* subject from the acknowledgement, so
    /// the two halves of one pull have to be joined somewhere; this is where.
    /// Keyed by all three parts because a host serves several downloads with
    /// different transfer ids, and the same index appears in every one of them.
    chunks: HashMap<(String, String, u64), tokio::sync::oneshot::Sender<Vec<u8>>>,
    /// Which conversation the UI is looking at, per host — the value the event
    /// task watches to move its `evt.` subscription between the narrow lane and
    /// the whole host.
    ///
    /// A `watch` rather than a plain field because the task is asleep on the
    /// socket and has to be woken by the change, and a sender rather than a
    /// value because the task owns the subscription: the runtime records the
    /// intent, the task is what acts on it.
    ///
    /// Kept after a disconnect, so reconnecting comes back on the lane the user
    /// left the window on rather than re-subscribing the whole host first.
    visible: HashMap<String, tokio::sync::watch::Sender<Option<String>>>,
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

/// One-shot injected race, for tests only: drop the connection between
/// `ensure_connected` and the lookup that follows it in [`request`].
///
/// The interleaving is real — a concurrent command's transport failure or a
/// user disconnect removes the entry — but it cannot be produced from outside
/// without racing, so it is injected the same way `remote_host::pairing`
/// injects a credential-write failure.
#[cfg(test)]
pub(crate) static INJECT_CONNECTION_LOST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
    // A pull in flight has no connection to complete on, so its waiter would
    // hang until the client's own timeout. Dropping the senders wakes it now.
    live.chunks.clear();
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
        .filter(|handle| !crate::runtime::task_finished(handle))
        .count()
}

/// How many retry loops are running.
#[cfg(test)]
pub(crate) async fn supervisor_count() -> usize {
    runtime().lock().await.supervisors.len()
}

/// Stop a host's stream task while leaving the connection in place.
///
/// The state a wedged or dead reader leaves behind: commands still work (the
/// host answers on the reply subject through this client's own request path),
/// but nothing is delivered from the subscriptions. That is exactly what a chunk
/// pull's timeout exists for, and it cannot be produced from outside.
#[cfg(test)]
pub(crate) async fn stop_stream_for_test(desktop_id: &str) {
    let task = runtime().lock().await.tasks.remove(desktop_id);
    if let Some(task) = task {
        task.abort();
    }
}

/// Close a live connection's socket without touching the retry state.///
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
    let mut creds = PeerCreds {
        pair_id: claimed.pair_id,
        desktop_id: parsed.desktop_id.clone(),
        device_id,
        nkey_seed: device
            .seed()
            .map_err(|error| crate::AppError::Message(format!("generate device NKey: {error}")))?,
        user_jwt: claimed.user_jwt,
        refresh_token: claimed.refresh_token,
        nats_url: claimed.nats_url.unwrap_or_default(),
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
    // The client-role claim carries only `nats_ws_url` — the TCP endpoint this
    // client needs is returned by the token endpoint alone. So a claim without
    // it is the *normal* case, resolved here with one refresh. Doing it before
    // the credential is stored, rather than lazily on connect, keeps what is on
    // disk complete: a later launch then never has to re-derive the endpoint to
    // reconnect.
    if creds.nats_url.is_empty() {
        let fresh = super::platform::refresh(&creds).await?;
        creds.user_jwt = fresh.user_jwt;
        creds.nats_url = fresh.nats_url;
        creds.nats_ws_url = fresh.nats_ws_url;
        creds.jwt_expires_at = fresh.jwt_expires_at;
    }
    let desktop_id = creds.desktop_id.clone();
    creds::upsert(creds)?;
    connect_with_emitter(&desktop_id, emitter).await
}

/// What the remote host shows in its "a new device paired" state. The platform
/// stores it verbatim, so it is the only place the host can learn that this
/// machine (rather than a phone) took the slot.
pub(crate) fn device_name() -> String {
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
    // What the UI is looking at, so a reconnection comes back on the lane the
    // user left it on instead of re-subscribing the whole host. Read (and the
    // receiver taken) before the subscribe below, which is the only ordering
    // that makes the value the same one the subscription is built from. The lock
    // is short and not held across a network round-trip.
    let (visible_now, visible_changes) = {
        let mut live = runtime().lock().await;
        let changes = live
            .visible
            .entry(desktop_id.to_string())
            .or_default()
            .clone();
        let now = changes.borrow().clone();
        (now, changes.subscribe())
    };
    // Subscribe before taking the runtime lock: subscription is a network
    // round-trip, and a failure there must not keep a usable connection out of
    // the map.
    let peer_session = connected.session;
    let stream = match emitter {
        Some(emitter) => {
            attach_stream(&peer_session, emitter, desktop_id, visible_now.as_deref()).await
        }
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
        if let Some(parts) = stream {
            live.tasks.insert(
                desktop_id.to_string(),
                spawn_event_stream(
                    desktop_id.to_string(),
                    peer_session.pair_id().to_string(),
                    peer_session.event_lane(),
                    visible_now,
                    visible_changes,
                    parts,
                ),
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
///
/// Also a process-lifetime task, and therefore spawned through
/// `crate::runtime` for the same reason as the stream above: a retry loop that
/// outlives its test would keep reconnecting under the next test's `HOME`.
pub(crate) fn spawn_supervisor(
    desktop_id: String,
    emitter: Option<Emitter>,
) -> crate::runtime::TaskHandle {
    crate::runtime::spawn(async move {
        let mut attempt = 0usize;
        loop {
            tokio::time::sleep(reconnect_delay(attempt)).await;
            // Already connected (something else reconnected while we waited).
            let connected = runtime().lock().await.live.contains_key(&desktop_id);
            if connected {
                return;
            }
            match resume(&desktop_id, emitter.clone()).await {
                Ok(()) => {
                    // Say so, and say it *after* the subscriptions are attached:
                    // a client told to catch up before the stream is live can
                    // still miss the events that arrive in between.
                    if let Some(emitter) = &emitter {
                        emitter(PeerEvent {
                            desktop_id: desktop_id.clone(),
                            kind: "resumed",
                            payload: json!({}),
                        });
                    }
                    return;
                }
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

/// The stream half of a connection: the traffic-key handle, the subscriptions
/// the event task drains and the sink they go to. Bundled rather than returned
/// as a tuple now that a fourth lane (the host-wide `state.>`) is among them.
struct StreamParts {
    channel: std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    subscriptions: super::session::Subscriptions,
    emitter: Emitter,
}

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
    visible: Option<&str>,
) -> Option<StreamParts> {
    match peer_session.subscribe(visible).await {
        Ok(subscriptions) => Some(StreamParts {
            channel: peer_session.channel_for_stream(),
            subscriptions,
            emitter,
        }),
        Err(error) => {
            eprintln!("remote_peer: could not subscribe for {desktop_id}: {error}");
            None
        }
    }
}

/// How many pushes one batch may carry before the loop yields, how many wire
/// bytes, and how long it may take.
///
/// Deliberately the mobile client's numbers (`mobile/src/remote/client.ts`):
/// both platforms read the same host lane, and a ceiling changed on one side
/// only re-introduces the per-token scheduler round trip the other side measured
/// away. At 60–90 msg/s this is one wakeup per ~8 ms instead of one per token.
const EVENT_BATCH_MAX: usize = 64;
const EVENT_BATCH_BYTES: usize = 256 * 1024;
const EVENT_BATCH_DEADLINE: Duration = Duration::from_millis(8);

/// How many pushes may wait for the decrypt/dispatch loop before the newest is
/// dropped.
///
/// The subscription itself is unbounded — `async_nats` buffers a fast publisher
/// without limit — so this is the client's own ceiling, and it is the host's
/// (`EVENT_QUEUE_CAPACITY`, the queue on the other end of the same pairing):
/// two ends of one link queued to the same depth. Overflow is never silent: the
/// drop is logged and a resync is pushed, because this client has no cursor to
/// fill a hole with, and a lost range would otherwise leave a transcript that
/// simply ends early.
const EVENT_BACKLOG_CAPACITY: usize = 4096;

/// The `PeerEvent.kind` a push is delivered under.
///
/// `evt.*` is the only lane that carries a conversation; `presence` is the
/// host's liveness tick; `state.*` is the host's host-wide lane, of which only
/// `state.events` is read as an event and `state.sessions`/`state.workspaces`
/// are forwarded as catalog updates; `catalog` is a decrypted snapshot. Chunks
/// are not a `PeerEvent` kind at all — they go to the pull waiting for them
/// ([`deliver_chunk`]).
const EVENT_LANE: &str = "event";
const PRESENCE_LANE: &str = "presence";
const STATE_LANE: &str = "state";
const CATALOG_LANE: &str = "catalog";
const TRANSFER_LANE: &str = "chunk";

/// One push taken off a subscription, before anything has been made of it.
///
/// The lane is resolved where the message is taken, because from there the
/// four subscriptions are one queue.
struct Push {
    kind: &'static str,
    message: async_nats::Message,
}

/// Wait for the next push on any lane, or `None` once a stream has ended.
async fn next_push(
    events: &mut async_nats::Subscriber,
    presence: &mut async_nats::Subscriber,
    state: &mut async_nats::Subscriber,
    transfers: &mut async_nats::Subscriber,
) -> Option<Push> {
    let (kind, message) = tokio::select! {
        message = events.next() => (EVENT_LANE, message),
        message = presence.next() => (PRESENCE_LANE, message),
        message = state.next() => (STATE_LANE, message),
        message = transfers.next() => (TRANSFER_LANE, message),
    };
    Some(Push {
        kind,
        message: message?,
    })
}

/// One push that has *already* arrived, without waiting for one that has not.
///
/// `async_nats`' `Subscriber` exposes no non-blocking read, so its `Stream` is
/// polled once here. That is cancel-safe — a `poll_next` that is not ready has
/// consumed nothing — so a poll that finds nothing loses nothing.
async fn ready_push(
    events: &mut async_nats::Subscriber,
    presence: &mut async_nats::Subscriber,
    state: &mut async_nats::Subscriber,
    transfers: &mut async_nats::Subscriber,
) -> Option<Push> {
    if let std::task::Poll::Ready(Some(message)) = futures::poll!(events.next()) {
        return Some(Push {
            kind: EVENT_LANE,
            message,
        });
    }
    if let std::task::Poll::Ready(Some(message)) = futures::poll!(presence.next()) {
        return Some(Push {
            kind: PRESENCE_LANE,
            message,
        });
    }
    if let std::task::Poll::Ready(Some(message)) = futures::poll!(state.next()) {
        return Some(Push {
            kind: STATE_LANE,
            message,
        });
    }
    if let std::task::Poll::Ready(Some(message)) = futures::poll!(transfers.next()) {
        return Some(Push {
            kind: TRANSFER_LANE,
            message,
        });
    }
    None
}

/// The three ceilings a batch of pushes yields on.
///
/// A value with an injected clock rather than a handful of locals, so each
/// boundary can be asserted without waiting out a real 8 ms.
struct BatchBudget {
    count: usize,
    bytes: usize,
    started: std::time::Instant,
}

impl BatchBudget {
    fn new(started: std::time::Instant) -> Self {
        Self {
            count: 0,
            bytes: 0,
            started,
        }
    }

    /// Record one delivery and say whether this batch is now due to yield.
    ///
    /// Bytes saturate rather than overflow: the ceiling is a few hundred KiB and
    /// a `usize` sum of message lengths cannot realistically wrap, but a wrapped
    /// sum would read as *under* the ceiling and keep a batch open forever.
    fn record(&mut self, bytes: usize, now: std::time::Instant) -> bool {
        self.count += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        self.count >= EVENT_BATCH_MAX
            || self.bytes >= EVENT_BATCH_BYTES
            || now.saturating_duration_since(self.started) >= EVENT_BATCH_DEADLINE
    }

    /// Begin the next batch at `now`.
    ///
    /// Without the reset a batch that ended on the count would still be over the
    /// byte or deadline ceiling when the next one starts, and every batch after
    /// the first would yield on its first delivery — the yield would keep
    /// happening but stop meaning anything.
    fn reset(&mut self, now: std::time::Instant) {
        self.count = 0;
        self.bytes = 0;
        self.started = now;
    }
}

/// A bounded queue of pushes waiting for the decrypt/dispatch loop.
///
/// Overflow drops the *newest* push and counts it, the same policy the host's
/// own event queue uses, so a burst that outruns the UI degrades identically on
/// both ends of the link.
struct Backlog<T> {
    items: std::collections::VecDeque<T>,
    capacity: usize,
    dropped: usize,
}

impl<T> Backlog<T> {
    fn new(capacity: usize) -> Self {
        Self {
            items: std::collections::VecDeque::new(),
            capacity: capacity.max(1),
            dropped: 0,
        }
    }

    /// Queue one push, or drop it when the queue is full. Says which happened.
    fn push(&mut self, item: T) -> bool {
        if self.items.len() >= self.capacity {
            self.dropped += 1;
            return false;
        }
        self.items.push_back(item);
        true
    }

    fn pop_front(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    /// Take the drop count accumulated since the last call, so the loop signals
    /// a resync once per episode of dropping rather than once per push.
    fn take_dropped(&mut self) -> usize {
        std::mem::take(&mut self.dropped)
    }
}

/// Drain the host's event, presence and file-transfer subscriptions.
///
/// Events and presence are pushed at the UI as `PeerEvent`s. Transfer bytes are
/// not: they are the other half of a pull the caller is waiting on, so they are
/// handed to that caller's inbox instead of being broadcast.
///
/// The traffic-key mutex is shared with the command path, so this task and a
/// concurrent request take the same short lock; neither holds it across an
/// `await`.
///
/// Pushes are taken in batches and handed over one at a time, with a yield
/// between batches ([`BatchBudget`], [`Backlog`]): the per-event contract the UI
/// consumes is unchanged, what changes is how often the loop wakes. A burst that
/// outruns the queue is dropped and answered with a resync rather than queued
/// without bound.
///
/// Every push's conversation is recovered from the subject it arrived on when
/// the payload does not carry one, because a lean host trims `sessionId` out of
/// the event envelope.
///
/// Spawned through `crate::runtime` rather than `tokio::spawn`: this is a
/// process-lifetime task, and the crate's spawner is what lets a test fixture
/// stop it before switching `HOME`. A raw spawn outlives the runtime that
/// started it — in the test harness that means one test's stream still running
/// while another test owns the process-global runtime and `HOME`.
fn spawn_event_stream(
    desktop_id: String,
    pair_id: String,
    lane: super::session::EventLane,
    // The value the subscription this task inherits was built for. Taken from
    // the connect path rather than read from `visible` here: the receiver may
    // already have moved on (the UI opens a conversation while a connection is
    // being set up), and reading *its* value would describe a lane that was
    // never subscribed — after which `changed()` reports nothing new and the
    // conversation would be served by the wide lane indefinitely.
    subscribed_with: Option<String>,
    visible: tokio::sync::watch::Receiver<Option<String>>,
    parts: StreamParts,
) -> crate::runtime::TaskHandle {
    spawn_event_stream_capped(
        desktop_id,
        pair_id,
        lane,
        subscribed_with,
        visible,
        parts,
        EVENT_BACKLOG_CAPACITY,
    )
}

/// [`spawn_event_stream`] with the backlog ceiling as an argument.
///
/// Production always passes [`EVENT_BACKLOG_CAPACITY`]; the parameter exists so a
/// test can exercise the *overflow* path without pushing 4096 sealed frames
/// (which the fake broker delivers at the rate TCP's small-write path allows —
/// tens of seconds). What the test then pins is the loop's reaction to a full
/// queue, not the size of the queue, which has its own assertion
/// (`the_batch_ceilings_are_the_mobile_clients`). The loop's behaviour is
/// otherwise byte-for-byte the shipping one.
fn spawn_event_stream_capped(
    desktop_id: String,
    pair_id: String,
    lane: super::session::EventLane,
    subscribed_with: Option<String>,
    visible: tokio::sync::watch::Receiver<Option<String>>,
    parts: StreamParts,
    backlog_capacity: usize,
) -> crate::runtime::TaskHandle {
    let StreamParts {
        channel,
        subscriptions:
            super::session::Subscriptions {
                mut events,
                mut presence,
                mut state,
                mut transfers,
            },
        emitter,
    } = parts;
    crate::runtime::spawn(async move {
        let mut visible = visible;
        // Which lane the events subscription is on, and therefore whether
        // `state.events` has to be read: on the wide lane every host-wide event
        // also arrives on its conversation's `evt.` subject, so reading both
        // would deliver it twice. Taken from the same `EventLane` the initial
        // `SUB` was built from, so the two cannot disagree.
        let mut installed = lane.for_visible(subscribed_with.as_deref());
        // Anything the UI asked for while this task was starting still has to be
        // applied: it was sent after the subscription above, so it is pending
        // here rather than waiting for the next change.
        let pending = visible.borrow_and_update().clone();
        apply_visible_lane(
            &lane,
            &mut installed,
            &mut events,
            pending.as_deref(),
            &desktop_id,
        )
        .await;
        /// The decrypted bytes, whatever they are: events are JSON, transfer
        /// chunks are a slice of a file. Kept as bytes so the chunk path does
        /// not have to pretend a binary payload is a document.
        fn open_bytes(
            channel: &std::sync::Mutex<future_remote_crypto::Channel>,
            subject: &str,
            payload: &[u8],
        ) -> Option<Vec<u8>> {
            channel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .open(subject, payload)
                .ok()
        }
        fn open(
            channel: &std::sync::Mutex<future_remote_crypto::Channel>,
            subject: &str,
            payload: &[u8],
        ) -> Option<Value> {
            // Same decoder as the command-reply path: the event lane is not
            // gzipped today, but one decoder is harder to get wrong than two.
            super::codec::decode_reply(&open_bytes(channel, subject, payload)?).ok()
        }

        /// Decrypt one push and hand it to whoever it belongs to.
        ///
        /// Chunks go to the pull waiting for them; everything else is a
        /// [`PeerEvent`]. A push that cannot be opened or parsed is dropped
        /// silently: it is either a forgery (a relay can inject one at will) or
        /// a shape this version does not know, and neither is the connection's
        /// fault, so there is nothing to report and nothing to retry.
        async fn handle_push(
            push: Push,
            channel: &std::sync::Mutex<future_remote_crypto::Channel>,
            desktop_id: &str,
            pair_id: &str,
            narrow: bool,
            emitter: &Emitter,
        ) {
            let Push { kind, message } = push;
            if kind == TRANSFER_LANE {
                // Parsed before decrypting: a malformed subject is dropped
                // without spending a decryption (and the traffic-key lock) on
                // it, and a relay that spams the transfer prefix cannot make
                // this client work to throw the messages away.
                let Some((transfer_id, index)) = parse_chunk_subject(&message.subject) else {
                    return;
                };
                let Some(bytes) = open_bytes(channel, &message.subject, &message.payload) else {
                    // Unauthenticated: a relay can inject one at will, and the
                    // connection is not at fault, so drop it.
                    return;
                };
                deliver_chunk(desktop_id, transfer_id, index, bytes).await;
                return;
            }
            // The host-wide lane. On the wide lane every host-wide event also
            // arrives on its conversation's `evt.` subject, so reading both
            // would deliver each approval card twice; while narrowed, this is
            // the only lane a non-visible conversation's approval or activity
            // can arrive on. The snapshots (`state.sessions`/`state.workspaces`)
            // are forwarded as catalog updates, not events.
            let (kind, domain) = if kind == STATE_LANE {
                match state_event_kind(&message.subject, pair_id) {
                    StateEventKind::Events if narrow => (EVENT_LANE, None),
                    StateEventKind::Sessions => (CATALOG_LANE, Some("sessions")),
                    StateEventKind::Workspaces => (CATALOG_LANE, Some("workspaces")),
                    _ => return,
                }
            } else {
                (kind, None)
            };
            let Some(mut payload) = open(channel, &message.subject, &message.payload) else {
                // Unparsable or unauthenticated; same reasoning as above.
                return;
            };
            // Each push's conversation is recovered from the subject it arrived
            // on when the payload does not carry one, because a lean host trims
            // `sessionId` out of the event envelope. `state.events` is the one
            // subject that cannot answer this — it names the host, not a
            // conversation — so a payload without a session there is emitted
            // as it came and dropped by the UI's own filter, which is the
            // honest outcome. A payload that already names its session is left
            // exactly as it came, so a client that declared nothing sees what it
            // always did.
            if payload.get("sessionId").and_then(Value::as_str).is_none() {
                if let (Some(session), Some(object)) = (
                    event_session_id(&message.subject, pair_id),
                    payload.as_object_mut(),
                ) {
                    object.insert("sessionId".to_string(), Value::String(session.to_string()));
                }
            }
            // Tag catalog snapshots with their domain so the UI does not have to
            // infer it from which top-level keys are present.
            if let (Some(domain), Some(object)) = (domain, payload.as_object_mut()) {
                object.insert("domain".to_string(), Value::String(domain.to_string()));
            }
            emitter(PeerEvent {
                desktop_id: desktop_id.to_string(),
                kind,
                payload,
            });
        }

        let mut backlog: Backlog<Push> = Backlog::new(backlog_capacity);
        let mut budget = BatchBudget::new(std::time::Instant::now());
        loop {
            // Park until something arrives: an idle link must cost nothing, and
            // the first push of a batch is the only one worth waiting for. The
            // user opening another conversation wakes the same loop, because
            // moving the events lane is the one thing that has to happen even
            // when the host is silent.
            let first = tokio::select! {
                push = next_push(&mut events, &mut presence, &mut state, &mut transfers) => {
                    let Some(push) = push else {
                        // Every stream ended: the socket is gone. The next status
                        // poll and the next command both notice; this task just
                        // stops.
                        return;
                    };
                    push
                }
                opened = visibility_change(&mut visible) => {
                    apply_visible_lane(
                        &lane,
                        &mut installed,
                        &mut events,
                        opened.as_deref(),
                        &desktop_id,
                    )
                    .await;
                    continue;
                }
            };
            backlog.push(first);
            // Take everything else the subscriptions have already buffered. Each
            // read is a poll rather than a wait, and a poll that finds nothing
            // returns *now* rather than parking, so this ends as soon as the
            // buffers are empty instead of holding the loop for a slow
            // publisher — and the queue's own ceiling is what stops a firehose
            // from growing it. (The mobile client's loop drains its subscription
            // the same way, until the SDK reports an empty queue.)
            while let Some(push) =
                ready_push(&mut events, &mut presence, &mut state, &mut transfers).await
            {
                backlog.push(push);
            }
            // Hand them over one at a time — the UI's contract is per event —
            // yielding between batches so one burst cannot starve everything
            // else on the runtime.
            while let Some(push) = backlog.pop_front() {
                let bytes = push.message.payload.len();
                handle_push(
                    push,
                    &channel,
                    &desktop_id,
                    &pair_id,
                    installed.narrow,
                    &emitter,
                )
                .await;
                if budget.record(bytes, std::time::Instant::now()) {
                    tokio::task::yield_now().await;
                    budget.reset(std::time::Instant::now());
                }
            }
            let dropped = backlog.take_dropped();
            if dropped > 0 {
                // The queue overflowed, so part of this conversation is gone and
                // no later push will repair it. `resumed` is the one signal the
                // UI already treats as "what the host sent is missing" (it
                // re-reads the page and the catalogue), which is exactly the
                // resync a drop needs — a new kind would be a contract change
                // the UI does not know how to act on.
                eprintln!(
                    "remote_peer: dropped {dropped} buffered pushes for {desktop_id}; re-syncing"
                );
                emitter(PeerEvent {
                    desktop_id: desktop_id.clone(),
                    kind: "resumed",
                    payload: json!({}),
                });
            }
        }
    })
}

/// Wait for the user to open a different conversation.
///
/// `changed()` answers `Err` once every sender is gone, which happens when the
/// runtime (and with it the map holding the senders) is replaced by a test
/// reset. Pending forever is the honest reading of that: the visibility can no
/// longer change, and a loop that kept re-polling would spin.
async fn visibility_change(
    visible: &mut tokio::sync::watch::Receiver<Option<String>>,
) -> Option<String> {
    match visible.changed().await {
        Ok(()) => visible.borrow_and_update().clone(),
        Err(_) => std::future::pending().await,
    }
}

/// Point the events lane at the conversation the UI is now on.
///
/// A new `SUB`, with the old subscriber dropped by the assignment: NATS Core has
/// no "change the subject back", and dropping the old one is also what stops the
/// previous conversation's pushes from arriving after the reader has moved on.
///
/// A no-op when the lane is already right, which is what makes a repeated
/// `set_visible_session` (a view re-rendering for reasons that have nothing to
/// do with navigation) harmless: re-subscribing would drop every event the host
/// published in the gap. The comparison is on the *subject*, so the two lanes
/// that both mean "the wide lane" cannot differ.
///
/// Events the host publishes for the new conversation between the old `SUB`
/// ending and the new one landing are lost — the window the host cannot know
/// about. The timeline re-reads the page when it opens a conversation, which is
/// what makes moving the lane safe.
async fn apply_visible_lane(
    lane: &super::session::EventLane,
    installed: &mut super::session::EventsSubscription,
    events: &mut async_nats::Subscriber,
    visible: Option<&str>,
    desktop_id: &str,
) {
    let wanted = lane.for_visible(visible);
    if wanted.subject == installed.subject {
        return;
    }
    match lane.subscribe(visible).await {
        Ok(subscriber) => {
            *events = subscriber;
            *installed = wanted;
        }
        Err(error) => {
            // The old lane stays installed and the next `resumed`/reconnect
            // rebuilds the stream; a failure here is not the connection's.
            eprintln!("remote_peer: could not move the event lane for {desktop_id}: {error}");
        }
    }
}

/// Hand one decrypted chunk to whoever is waiting for it.
///
/// A chunk nobody is waiting for is dropped rather than queued: the host only
/// sends bytes in answer to a pull, so an unmatched one is either a duplicate
/// from a retry or a reply to a pull that already timed out. Buffering it would
/// keep file bytes in memory for a download the caller has abandoned.
async fn deliver_chunk(desktop_id: &str, transfer_id: String, index: u64, bytes: Vec<u8>) {
    let waiter =
        runtime()
            .lock()
            .await
            .chunks
            .remove(&(desktop_id.to_string(), transfer_id, index));
    if let Some(waiter) = waiter {
        // A closed receiver means the pull gave up; the bytes are then unwanted.
        let _ = waiter.send(bytes);
    }
}

/// The conversation an `evt.>` subject names, or `None` when the subject is not
/// one of this pairing's event subjects.
///
/// The host publishes an event on `p.{pair}.evt.{session}`
/// (`remote::publisher::publish_event`), and this client subscribes with exactly
/// `p.{pair}.evt.>` (`session.rs::subscribe`), so the session is whatever
/// follows that prefix. Slicing the *known prefix* rather than splitting into
/// fixed fields is what the phone does, and it stays right if a pair or session
/// id ever contained a dot: the tokens the host joined are the ones we strip.
///
/// A subject that does not carry the prefix yields `None` rather than a guessed
/// session. Every UI listener drops an event without a `sessionId`, which is
/// the honest outcome (the event is not ours to file); inventing one would file
/// it under another conversation.
pub(crate) fn event_session_id<'a>(subject: &'a str, pair_id: &str) -> Option<&'a str> {
    let rest = subject.strip_prefix("p.")?;
    let rest = rest.strip_prefix(pair_id)?;
    let session = rest.strip_prefix(".evt.")?;
    (!session.is_empty()).then_some(session)
}

/// Which kind of host-wide lane a `state.>` subject names.
///
/// The host publishes three things under `state.>`: the event mirror, the
/// sessions snapshot, and the workspaces snapshot. Only `state.events` is read
/// as an event (and only while the event lane is narrowed); the snapshots are
/// forwarded as catalog updates.
enum StateEventKind {
    Events,
    Sessions,
    Workspaces,
    Other,
}

fn state_event_kind(subject: &str, pair_id: &str) -> StateEventKind {
    if subject == format!("p.{pair_id}.state.events") {
        StateEventKind::Events
    } else if subject == format!("p.{pair_id}.state.sessions") {
        StateEventKind::Sessions
    } else if subject == format!("p.{pair_id}.state.workspaces") {
        StateEventKind::Workspaces
    } else {
        StateEventKind::Other
    }
}

/// `transfer_id` and chunk index from a `xfer.down` subject.
///
/// Returns `None` for anything else, including a subject from another pair:
/// the host's own layout is `p.{pair}.xfer.down.{transfer}.chunk.{index}`, and a
/// chunk whose shape does not match that is not something to guess at.
pub(crate) fn parse_chunk_subject(subject: &str) -> Option<(String, u64)> {
    let parts: Vec<&str> = subject.split('.').collect();
    let ["p", _pair, "xfer", "down", transfer_id, "chunk", index] = parts.as_slice() else {
        return None;
    };
    index
        .parse::<u64>()
        .ok()
        .map(|index| ((*transfer_id).to_string(), index))
}

/// Record which conversation the UI is looking at, so the live lane can be
/// narrowed to it (`selective_events_v1`).
///
/// A hint, not a command: it returns as soon as the intent is recorded, whether
/// or not a host is connected, and whether or not the host confirmed the
/// capability. Nothing here fails, because there is nothing the UI could do
/// about a failure — the wide lane is the fallback and it is already running.
///
/// `None` (or an id that is not a plain token, or a host that never confirmed
/// the capability) means the wide lane, which is what keeps "no conversation is
/// open" and "this host cannot narrow" the same case.
pub(crate) async fn set_visible_session(desktop_id: &str, session_id: Option<&str>) {
    let mut live = runtime().lock().await;
    let changes = live.visible.entry(desktop_id.to_string()).or_default();
    let next = session_id.map(str::to_string);
    // No-op when it has not moved: a view that re-renders (the timeline hook
    // re-runs its effect on every dependency change) must not make the event
    // task drop and re-open its subscription for the conversation it is already
    // on — that would lose the pushes published in between.
    if *changes.borrow() != next {
        changes.send_replace(next);
    }
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
///
/// **This client reads pages in chunks.** `chunkedRead` is set here rather than
/// by each caller because it is a property of the client, not of a request: the
/// host serves any reply over its page threshold as an immutable snapshot split
/// into ≤192 KiB chunks ([`reassemble`] puts it back together), which is what
/// keeps a page of any size deliverable. Without it every read has to fit one
/// reply, and a conversation whose newest exchange is large — one long agent run
/// is one exchange — is answered with `remote_reply_too_large` instead of
/// opening.
pub(crate) async fn request(
    desktop_id: &str,
    command: Value,
    lane: &str,
) -> Result<Value, crate::AppError> {
    // Read before the command is consumed: the chunk fetches have to name the
    // same session and run as the page they are completing.
    let owner = ChunkOwner::from(&command);
    let reply = send_request(desktop_id, command, lane).await?;
    match read_chunk_of(&reply) {
        // A chunked reply: put the page back together before returning it, so no
        // caller has to know a page can arrive in pieces.
        Some(chunk) => reassemble(desktop_id, lane, chunk, &owner).await,
        None => Ok(reply),
    }
}

/// The fields a chunk fetch has to repeat from the command that started the page.
///
/// The host scopes a page to the bridge, session and run that asked for it and
/// refuses a chunk asked for by anything else (`remote_read_owner_mismatch`), so
/// the fetch repeats them verbatim. A command that names neither — a catalogue
/// read — is fetched with neither, which is exactly what the host recorded for
/// it. The bridge instance is not here: it travels with the session itself, so
/// every request on the same connection carries it.
struct ChunkOwner {
    session: Option<String>,
    run: Option<String>,
}

impl ChunkOwner {
    fn from(command: &Value) -> Self {
        let field = |key: &str| command.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            session: field("sessionId"),
            run: field("runId"),
        }
    }
}

/// One round trip: the command and its reply, unreassembled.
///
/// Split from [`request`] because chunk fetching goes through it directly: a
/// `get_read_chunk` reply *is* a chunk, and routing it back through the
/// reassembling wrapper would make the two functions mutually recursive.
async fn send_request(
    desktop_id: &str,
    mut command: Value,
    lane: &str,
) -> Result<Value, crate::AppError> {
    ensure_connected(desktop_id).await?;
    if let Some(object) = command.as_object_mut() {
        object
            .entry("id")
            .or_insert_with(|| json!(crate::store::create_id("cmd")));
        // Never on the chunk fetch itself: the host pages the page-producing
        // commands, and this one's reply *is* a chunk.
        if object.get("type").and_then(Value::as_str) != Some("get_read_chunk") {
            object.insert("chunkedRead".into(), json!(true));
        }
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
    #[cfg(test)]
    if INJECT_CONNECTION_LOST.swap(false, std::sync::atomic::Ordering::Relaxed) {
        // The real interleaving this guards: a concurrent command's transport
        // failure, or the user pressing Disconnect, removes the entry between
        // the connect above and the lookup below. Forced here because it cannot
        // be produced from outside without racing.
        live.live.remove(desktop_id);
    }
    let Some(session) = live.live.get_mut(desktop_id) else {
        return Err(crate::AppError::Message("peer_not_connected".into()));
    };
    let subject = session.command_subject(&key);
    let result = session
        .request(&subject, command, session::COMMAND_TIMEOUT)
        .await;
    if let Err(error) = &result {
        on_transport_failure(&mut live, desktop_id, error);
    }
    result
}

/// The chunk a reply carries, when the host split the page it answered with.
///
/// The shape is the host's (`remote_host/read_pages.rs`): a page over the host's
/// threshold is stored as an immutable snapshot and the reply names the first
/// chunk. Its *presence* — not a flag this client set — is what says the reply is
/// a fragment, which is also what makes an older host's un-paged reply pass
/// through untouched.
fn read_chunk_of(reply: &Value) -> Option<&Value> {
    reply.get("readChunk").filter(|chunk| chunk.is_object())
}

/// How large a reassembled page may be.
///
/// The host's own snapshot ceiling (`MAX_SNAPSHOT_BYTES`), restated because this
/// is the end that holds the bytes: a host that answered with more is refused
/// rather than buffered.
const MAX_PAGE_BYTES: u64 = 16 * 1024 * 1024;
/// The host's chunk size, from which every offset is derived.
const PAGE_CHUNK_BYTES: usize = 192 * 1024;
/// How many chunks are fetched at once.
///
/// The host serves indexed chunks independently, so a large page costs one round
/// trip per chunk however they are ordered. Sequential pulls make that latency
/// visible — a 4 MiB page is twenty — and four in flight amortizes it without
/// opening a burst the host has to buffer.
const PAGE_CHUNK_CONCURRENCY: usize = 4;

/// Fetch the rest of a page's chunks and decode it.
async fn reassemble(
    desktop_id: &str,
    lane: &str,
    first: &Value,
    owner: &ChunkOwner,
) -> Result<Value, crate::AppError> {
    let id = chunk_string(first, "id")?;
    let total = chunk_u64(first, "totalBytes")?;
    if total > MAX_PAGE_BYTES {
        return Err(crate::AppError::Message("remote_read_too_large".into()));
    }
    let total = usize::try_from(total).map_err(|_| page_shape_error())?;
    let mut bytes = vec![0u8; total];
    store_chunk(&mut bytes, first, &id, total, 0)?;
    let mut next = chunk_u64(first, "nextOffset")? as usize;
    while next < total {
        let offsets: Vec<usize> = (0..PAGE_CHUNK_CONCURRENCY)
            .map(|index| next + index * PAGE_CHUNK_BYTES)
            .filter(|offset| *offset < total)
            .collect();
        let mut fetched = Vec::with_capacity(offsets.len());
        for offset in offsets {
            fetched.push((
                offset,
                fetch_chunk(desktop_id, lane, &id, offset, owner).await?,
            ));
        }
        for (offset, reply) in fetched {
            let chunk = read_chunk_of(&reply).ok_or_else(page_shape_error)?;
            store_chunk(&mut bytes, chunk, &id, total, offset)?;
            next = (offset + PAGE_CHUNK_BYTES).min(total);
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| page_shape_error())
}

/// One `get_read_chunk` round trip, on the lane of the page it belongs to.
async fn fetch_chunk(
    desktop_id: &str,
    lane: &str,
    id: &str,
    offset: usize,
    owner: &ChunkOwner,
) -> Result<Value, crate::AppError> {
    let mut command = json!({"type": "get_read_chunk", "replyId": id, "offset": offset});
    if let Some(session) = &owner.session {
        command["sessionId"] = json!(session);
    }
    if let Some(run) = &owner.run {
        command["runId"] = json!(run);
    }
    send_request(desktop_id, command, lane).await
}

/// Copy a chunk's decoded bytes into the page, checking it is the chunk it claims.
///
/// Every field is verified against the snapshot the first reply pinned: a chunk
/// from another snapshot (a re-read that raced this one), one at another offset,
/// or a short one while the page continues would all leave the assembled bytes
/// wrong — silently, because the result can still be valid JSON. Refusing is the
/// only honest option: a half-page that decodes is worse than an error the reader
/// can retry.
fn store_chunk(
    bytes: &mut [u8],
    chunk: &Value,
    id: &str,
    total: usize,
    expected_offset: usize,
) -> Result<(), crate::AppError> {
    if chunk_string(chunk, "id")? != id
        || chunk_u64(chunk, "totalBytes")? as usize != total
        || chunk_u64(chunk, "offset")? as usize != expected_offset
    {
        return Err(page_shape_error());
    }
    let part = URL_SAFE_NO_PAD
        .decode(chunk_string(chunk, "data")?.as_bytes())
        .map_err(|_| page_shape_error())?;
    let next = chunk_u64(chunk, "nextOffset")? as usize;
    if part.is_empty()
        || part.len() > PAGE_CHUNK_BYTES
        || next != expected_offset + part.len()
        || next > total
        // A short chunk with more page to come would leave a hole.
        || (next < total && part.len() != PAGE_CHUNK_BYTES)
    {
        return Err(page_shape_error());
    }
    bytes
        .get_mut(expected_offset..next)
        .ok_or_else(page_shape_error)?
        .copy_from_slice(&part);
    Ok(())
}

fn chunk_string(chunk: &Value, key: &str) -> Result<String, crate::AppError> {
    chunk
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(page_shape_error)
}

fn chunk_u64(chunk: &Value, key: &str) -> Result<u64, crate::AppError> {
    chunk
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(page_shape_error)
}

fn page_shape_error() -> crate::AppError {
    crate::AppError::Message("remote_read_invalid_chunk".into())
}

/// Note a transport failure and make sure the link is being retried.
///
/// Shared by the session-keyed command lane and the transfer lane: both talk to
/// the same socket, so both have to leave the same state behind when it dies. A
/// second copy is a second place for the recovery to drift.
fn on_transport_failure(live: &mut Runtime, desktop_id: &str, error: &crate::AppError) {
    // Only a transport fault means the socket is gone. A host that answered and
    // refused is still connected.
    if !matches!(error, crate::AppError::RemoteTransport(_)) {
        return;
    }
    // A dead socket must not keep looking connected on the next poll.
    live.errors
        .insert(desktop_id.to_string(), error.to_string());
    live.live.remove(desktop_id);
    // The connection was healthy and then was not: exactly the case automatic
    // recovery is for. An explicit disconnect takes the supervisor with it, so
    // this cannot resurrect a stopped link.
    if !live.supervisors.contains_key(desktop_id) {
        let supervisor = spawn_supervisor(desktop_id.to_string(), None);
        live.supervisors.insert(desktop_id.to_string(), supervisor);
    }
}

/// Record one chunk of an upload.
///
/// The mirror of `pull_chunk`, and simpler because there is nothing to join: the
/// chunk *is* the request body, and the reply is the host's acknowledgement.
pub(crate) async fn put_chunk(
    desktop_id: &str,
    upload_id: &str,
    index: u64,
    bytes: &[u8],
) -> Result<(), crate::AppError> {
    let mut live = runtime().lock().await;
    let Some(session) = live.live.get_mut(desktop_id) else {
        return Err(crate::AppError::Message("peer_not_connected".into()));
    };
    let subject = session.transfer_chunk_subject(upload_id, index);
    // The session budget, not a transfer one: the host writes this chunk to disk
    // before it answers, so a short budget here would report a slow disk as a
    // dead socket.
    let result = session
        .request_bytes(&subject, bytes, session::COMMAND_TIMEOUT)
        .await;
    if let Err(error) = &result {
        on_transport_failure(&mut live, desktop_id, error);
    }
    result.map(|_| ())
}

/// Where a chunk pull is requested. Built from the live session so the subject
/// matches *this* pairing rather than whatever the credential file says.
pub(crate) async fn transfer_pull_subject(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<String, crate::AppError> {
    let live = runtime().lock().await;
    let Some(session) = live.live.get(desktop_id) else {
        return Err(crate::AppError::Message("peer_not_connected".into()));
    };
    Ok(session.transfer_pull_subject(transfer_id, index))
}

/// Hold a place for the bytes of one chunk.
pub(crate) async fn register_chunk(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
    sender: tokio::sync::oneshot::Sender<Vec<u8>>,
) {
    runtime().lock().await.chunks.insert(
        (desktop_id.to_string(), transfer_id.to_string(), index),
        sender,
    );
}

/// Give up a place for one chunk.
pub(crate) async fn forget_chunk(desktop_id: &str, transfer_id: &str, index: u64) {
    runtime()
        .lock()
        .await
        .chunks
        .remove(&(desktop_id.to_string(), transfer_id.to_string(), index));
}

/// The bytes of one chunk: ask, and join the two halves of the answer.///
/// The bytes arrive as a publish on a different subject from the pull's
/// acknowledgement, so this is the only place the two are brought together —
/// which is also why the waiter is registered *before* the request: the host
/// publishes the chunk first, and a waiter added afterwards would miss it.
pub(crate) async fn pull_chunk(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<Vec<u8>, crate::AppError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    register_chunk(desktop_id, transfer_id, index, sender).await;
    let subject = match transfer_pull_subject(desktop_id, transfer_id, index).await {
        Ok(subject) => subject,
        Err(error) => {
            forget_chunk(desktop_id, transfer_id, index).await;
            return Err(error);
        }
    };
    // The *request* keeps a real budget, and it is not the one above. It makes
    // the host read the chunk off disk, publish it and flush — so a short budget
    // here fails the command, which the transport layer reads as a dead socket:
    // the connection is dropped and the retry reports `peer_not_connected`, i.e.
    // a slow disk on a busy machine looks exactly like a network fault. The
    // shortened test budget applies only to waiting for the bytes, which the
    // host has already published by the time it acknowledges.
    if let Err(error) = request_at(desktop_id, json!({}), &subject, session::COMMAND_TIMEOUT).await
    {
        forget_chunk(desktop_id, transfer_id, index).await;
        return Err(error);
    }
    match tokio::time::timeout(chunk_pull_timeout(), receiver).await {
        Ok(Ok(bytes)) => Ok(bytes),
        // Nothing arrived: either the sender was dropped (the connection went
        // away, or the runtime was cleared) or the host acknowledged a publish
        // that never reached this subscription. The two are indistinguishable
        // from here and both are transient, so they share one message — and the
        // caller retries either way.
        _ => {
            forget_chunk(desktop_id, transfer_id, index).await;
            Err(crate::AppError::RemoteTransport(
                "remote_download_chunk_missing".into(),
            ))
        }
    }
}

/// The timeout for one chunk pull, matching the phone's own.
///
/// Milliseconds under test, so the suite does not spend 15 s per attempt waiting
/// out an arm that exists to catch a host which acknowledged a publish that
/// never arrived. The *shape* being exercised is the real one either way.
fn chunk_pull_timeout() -> Duration {
    #[cfg(test)]
    const TIMEOUT: Duration = Duration::from_millis(500);
    #[cfg(not(test))]
    const TIMEOUT: Duration = Duration::from_secs(15);
    TIMEOUT
}

/// One command on an explicit subject, for the routes that are not the
/// session-keyed command lane (file transfer addresses a transfer id).
pub(crate) async fn request_at(
    desktop_id: &str,
    mut command: Value,
    subject: &str,
    timeout: Duration,
) -> Result<Value, crate::AppError> {
    ensure_connected(desktop_id).await?;
    if let Some(object) = command.as_object_mut() {
        object
            .entry("id")
            .or_insert_with(|| json!(crate::store::create_id("cmd")));
    }
    let mut live = runtime().lock().await;
    #[cfg(test)]
    if INJECT_CONNECTION_LOST.swap(false, std::sync::atomic::Ordering::Relaxed) {
        // The same interleaving the session-keyed lane guards against: a
        // concurrent command's transport failure, or the user pressing
        // Disconnect, removes the entry between the connect above and the
        // lookup below.
        live.live.remove(desktop_id);
    }
    let Some(session) = live.live.get_mut(desktop_id) else {
        return Err(crate::AppError::Message("peer_not_connected".into()));
    };
    let result = session.request(subject, command, timeout).await;
    if let Err(error) = &result {
        on_transport_failure(&mut live, desktop_id, error);
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
    // A reply that is not an object violates the host's own contract. Reported
    // rather than asserted: the field comes from a *remote* peer, and panicking
    // on remote input would hand a broken or hostile host a way to take the app
    // down.
    let object = data
        .as_object_mut()
        .ok_or_else(|| crate::AppError::Message("peer_reply_not_an_object".into()))?;
    object.insert("desktopId".into(), json!(desktop_id));
    // `null` rather than omitted when the connection has already gone: the row
    // still came from this host, and a reader can tell "no live pairing" from
    // "field missing".
    object.insert("pairId".into(), pair_id.map_or(Value::Null, |id| json!(id)));
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
            attach_stream(&session, silent_emitter(), &desktop_id, None)
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
            attach_stream(&session, silent_emitter(), &desktop_id, None)
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
        // `err()` rather than `expect_err`: the ok arm holds a live connection,
        // which has no `Debug` (and should not grow one just for a test).
        let error = super::super::session::connect(&broken)
            .await
            .err()
            .expect("an invalid seed cannot sign");
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

    /// A catalogue reply that is not an object is a protocol violation, not a
    /// shape to invent — and it must not panic, because the reply comes from a
    /// remote peer: a broken or hostile host could otherwise take the app down.
    #[tokio::test]
    async fn a_catalogue_reply_that_is_not_an_object_is_refused() {
        let _home = crate::remote::test_support::HomeGuard::new("peer-stamp-non-object");
        reset_for_test().await;
        for reply in [
            serde_json::json!([1, 2, 3]),
            serde_json::json!("a string"),
            serde_json::Value::Null,
        ] {
            let error = stamp_live("desktop_any", reply.clone())
                .await
                .expect_err("a non-object reply cannot be stamped");
            assert_eq!(error.to_string(), "peer_reply_not_an_object", "{reply}");
        }
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

    /// The token chain the host joins (`p.{pair}.evt.{session}`) is what decides
    /// which conversation a push belongs to, so it is read back rather than
    /// assumed.
    #[test]
    fn an_event_subject_names_its_session() {
        assert_eq!(
            event_session_id("p.pair_1.evt.sess_a", "pair_1"),
            Some("sess_a")
        );
        // Ids are opaque: a dotted session is one token chain, not a field to
        // split, and the pair prefix is stripped as a string for the same
        // reason.
        assert_eq!(
            event_session_id("p.pair_1.evt.sess.with.dots", "pair_1"),
            Some("sess.with.dots")
        );
    }

    /// Everything else on the wire is not an event subject, and the answer for
    /// it is `None`: an `evt.>` subscription never delivers these, but a caller
    /// that guessed would file a presence tick under a conversation.
    #[test]
    fn a_subject_that_is_not_an_event_is_refused() {
        for subject in [
            "p.pair_1.presence",
            "p.pair_1.state.events",
            "p.pair_1.xfer.down.download_9.chunk.3",
            // Another pairing's event lane: the prefix must match whole tokens,
            // so a pair id that is merely a prefix of another does not match.
            "p.pair_10.evt.sess_a",
            // No session token at all (`evt.>` requires one).
            "p.pair_1.evt.",
            "p.pair_1.evt",
            "",
        ] {
            assert_eq!(
                event_session_id(subject, "pair_1"),
                None,
                "subject {subject}"
            );
        }
    }

    /// Only the `state.events` mirror of the host-wide lane is read as an event;
    /// the snapshots on the same wildcard are forwarded as catalog updates, and
    /// everything else is ignored.
    #[test]
    fn only_the_state_events_mirror_is_read_from_the_host_wide_lane() {
        assert!(matches!(
            state_event_kind("p.pair_1.state.events", "pair_1"),
            StateEventKind::Events
        ));
        assert!(matches!(
            state_event_kind("p.pair_1.state.sessions", "pair_1"),
            StateEventKind::Sessions
        ));
        assert!(matches!(
            state_event_kind("p.pair_1.state.workspaces", "pair_1"),
            StateEventKind::Workspaces
        ));
        for subject in [
            "p.pair_1.state.version",
            // Another pairing's host-wide lane.
            "p.pair_10.state.events",
            // The events lane proper, which is read through its own subscription.
            "p.pair_1.evt.sess_a",
            "",
        ] {
            assert!(
                matches!(state_event_kind(subject, "pair_1"), StateEventKind::Other),
                "subject {subject}"
            );
        }
    }

    // ── the batch budget and the backlog ────────────────────────────────────

    /// The three ceilings are the mobile client's, because both platforms read
    /// the same host lane: a number changed here alone re-introduces on this
    /// side the per-token wakeup the other side measured away.
    #[test]
    fn the_batch_ceilings_are_the_mobile_clients() {
        assert_eq!(EVENT_BATCH_MAX, 64);
        assert_eq!(EVENT_BATCH_BYTES, 256 * 1024);
        assert_eq!(EVENT_BATCH_DEADLINE, std::time::Duration::from_millis(8));
        assert_eq!(EVENT_BACKLOG_CAPACITY, 4096);
    }

    /// ① The count ceiling: 64 small pushes end the batch on the 64th.
    #[test]
    fn a_batch_yields_at_the_count_ceiling() {
        let now = std::time::Instant::now();
        let mut budget = BatchBudget::new(now);
        for delivered in 1..EVENT_BATCH_MAX {
            assert!(
                !budget.record(1, now),
                "push {delivered} is before the ceiling"
            );
        }
        assert!(
            budget.record(1, now),
            "the {EVENT_BATCH_MAX}th push ends the batch"
        );
    }

    /// ② The byte ceiling: one large push ends the batch long before the count
    /// does — a burst of big events is what makes the difference visible.
    #[test]
    fn a_batch_yields_at_the_byte_ceiling() {
        let now = std::time::Instant::now();
        let mut budget = BatchBudget::new(now);
        assert!(
            !budget.record(EVENT_BATCH_BYTES - 1, now),
            "one byte short of the ceiling is still inside the batch"
        );
        assert!(budget.record(1, now), "the ceiling itself is due");
    }

    /// ③ The deadline: a slow trickle of small pushes still hands control back,
    /// or a quiet link would hold one batch open for as long as it stayed quiet.
    #[test]
    fn a_batch_yields_at_the_deadline() {
        let now = std::time::Instant::now();
        let started = now - EVENT_BATCH_DEADLINE - std::time::Duration::from_millis(1);
        let mut budget = BatchBudget::new(started);
        assert!(
            budget.record(1, now),
            "the first push after the deadline ends the batch"
        );
    }

    /// A batch that ended on the count must not make the *next* batch yield on
    /// its first delivery: the reset is what keeps every batch after the first
    /// meaningful.
    #[test]
    fn resetting_starts_the_next_batch_empty() {
        let now = std::time::Instant::now();
        let mut budget = BatchBudget::new(now);
        for _ in 0..EVENT_BATCH_MAX {
            budget.record(1, now);
        }
        budget.reset(now);
        assert!(
            !budget.record(1, now),
            "the next batch starts with one delivery, not a full budget"
        );
    }

    /// The backlog's ceiling drops the newest push and counts it once, so the
    /// loop can answer with a single resync instead of one per lost push.
    #[test]
    fn a_full_backlog_drops_the_newest_and_counts_it() {
        let mut backlog = Backlog::new(2);
        assert!(backlog.push("a"));
        assert!(backlog.push("b"));
        assert!(!backlog.push("c"), "past the ceiling the push is dropped");
        assert!(!backlog.push("d"));
        assert_eq!(backlog.take_dropped(), 2);
        assert_eq!(
            backlog.take_dropped(),
            0,
            "a drop is reported once, not once per call"
        );
        // What was queued still comes out in order: the cap drops the newest,
        // never the oldest, so the UI never sees a reorder on the way in.
        assert_eq!(backlog.pop_front(), Some("a"));
        assert_eq!(backlog.pop_front(), Some("b"));
        assert_eq!(backlog.pop_front(), None);
    }

    /// A queue sized zero would drop everything; the floor keeps a
    /// misconfiguration from turning every push into a resync.
    #[test]
    fn a_zero_capacity_backlog_still_holds_one() {
        let mut backlog = Backlog::new(0);
        assert!(backlog.push("a"));
        assert!(!backlog.push("b"));
        assert_eq!(backlog.pop_front(), Some("a"));
    }
}

/// Seams the coverage tests need that the shipped test surface does not expose.
///
/// Three of the four behaviours pinned in `runtime_tests.rs` live inside the
/// event-stream task or in functions private to this module, and one of them
/// needs a body the *current* host no longer writes (a lean host that trimmed
/// `sessionId`). Reaching them through `connect_with_emitter` would either be
/// impossible or race a live reader, so the harness builds a session whose
/// traffic key the test holds in the **host** direction and drives the private
/// functions directly.
#[cfg(test)]
pub(crate) mod cov_harness {
    use super::*;
    use crate::remote::test_support::{nats_connect, FakeNats};
    use crate::remote_peer::session::{EventLane, EventsSubscription, PeerSession, Subscriptions};

    /// A matched Noise channel pair from a real `XXpsk0` handshake: the first
    /// value is the **client** half (what a `PeerSession` holds), the second the
    /// **host** half, so a test can seal a push with the key the client opens it
    /// with. One `Channel` cannot open what it sealed — the send and receive keys
    /// differ — which is why the pair, not a single channel, is returned.
    pub(crate) fn noise_pair(
        pair_id: &str,
    ) -> (future_remote_crypto::Channel, future_remote_crypto::Channel) {
        let (client_private, _) =
            future_remote_crypto::generate_identity().expect("client identity");
        let (host_private, _) = future_remote_crypto::generate_identity().expect("host identity");
        let secret = [9_u8; 32];
        let prologue = future_remote_crypto::prologue(pair_id, "desktop_cov").expect("prologue");
        let mut initiator = future_remote_crypto::Handshake::new(
            future_remote_crypto::Pattern::Pair,
            true,
            &client_private,
            None,
            Some(&secret),
            &prologue,
        )
        .expect("initiator");
        let mut responder = future_remote_crypto::Handshake::new(
            future_remote_crypto::Pattern::Pair,
            false,
            &host_private,
            None,
            Some(&secret),
            &prologue,
        )
        .expect("responder");
        // The XXpsk0 legs, the same three the real handshake runs.
        responder
            .read(&initiator.write(b"").expect("leg 1"))
            .expect("leg 1");
        initiator
            .read(&responder.write(b"").expect("leg 2"))
            .expect("leg 2");
        responder
            .read(&initiator.write(b"").expect("leg 3"))
            .expect("leg 3");
        (
            initiator.finish().expect("client channel"),
            responder.finish().expect("host channel"),
        )
    }

    /// A broker and a client connected to it, ready for a `PeerSession`.
    pub(crate) async fn broker_and_client() -> (FakeNats, async_nats::Client) {
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        (nats, client)
    }

    /// A session whose channel the test owns, so it can also hold the host half.
    pub(crate) fn session_for(
        pair_id: &str,
        client: async_nats::Client,
        channel: future_remote_crypto::Channel,
    ) -> PeerSession {
        PeerSession::for_test(
            pair_id,
            client,
            std::sync::Arc::new(std::sync::Mutex::new(channel)),
            "bridge_cov",
        )
    }

    /// Seal `body` with the **host** key and deliver it on `subject`, exactly as
    /// the host's own publisher does (`context` is the subject).
    pub(crate) fn inject_sealed(
        nats: &FakeNats,
        host: &std::sync::Mutex<future_remote_crypto::Channel>,
        subject: &str,
        body: &serde_json::Value,
    ) {
        let bytes = serde_json::to_vec(body).expect("a Value always serializes");
        let wire = host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .seal(subject, &bytes)
            .expect("seal as the host");
        nats.inject(subject, None, wire);
    }

    /// Subscribe and start the stream loop for a session the test owns.
    pub(crate) async fn spawn_stream(
        session: &PeerSession,
        desktop_id: &str,
        emitter: Emitter,
    ) -> (
        crate::runtime::TaskHandle,
        tokio::sync::watch::Sender<Option<String>>,
    ) {
        let subscriptions = session.subscribe(None).await.expect("subscribe");
        spawn_stream_with(session, subscriptions, desktop_id, emitter)
    }

    /// Start the stream loop on subscriptions the test already took, so a burst
    /// can be buffered *before* anything drains it.
    pub(crate) fn spawn_stream_with(
        session: &PeerSession,
        subscriptions: Subscriptions,
        desktop_id: &str,
        emitter: Emitter,
    ) -> (
        crate::runtime::TaskHandle,
        tokio::sync::watch::Sender<Option<String>>,
    ) {
        spawn_stream_with_capacity(
            session,
            subscriptions,
            desktop_id,
            emitter,
            BACKLOG_CAPACITY,
        )
    }

    /// [`spawn_stream_with`] with the backlog ceiling as an argument, so the
    /// overflow path is reachable without pushing 4096 sealed frames through the
    /// fake broker. Production keeps [`BACKLOG_CAPACITY`]; only the size differs.
    pub(crate) fn spawn_stream_with_capacity(
        session: &PeerSession,
        subscriptions: Subscriptions,
        desktop_id: &str,
        emitter: Emitter,
        backlog_capacity: usize,
    ) -> (
        crate::runtime::TaskHandle,
        tokio::sync::watch::Sender<Option<String>>,
    ) {
        let parts = StreamParts {
            channel: session.channel_for_stream(),
            subscriptions,
            emitter,
        };
        let (sender, receiver) = tokio::sync::watch::channel(None);
        let handle = spawn_event_stream_capped(
            desktop_id.to_string(),
            session.pair_id().to_string(),
            session.event_lane(),
            None,
            receiver,
            parts,
            backlog_capacity,
        );
        (handle, sender)
    }

    /// A second client on the same broker, for a test that needs one connection
    /// to stay live while another is drained.
    pub(crate) async fn another_client(nats: &FakeNats) -> async_nats::Client {
        nats_connect(nats).await
    }

    /// The `visibility_change` the loop awaits, for a test that owns the watch.
    pub(crate) async fn visibility_change_for_test(
        visible: &mut tokio::sync::watch::Receiver<Option<String>>,
    ) -> Option<String> {
        visibility_change(visible).await
    }

    /// The lane move the loop performs when the user opens a conversation.
    pub(crate) async fn apply_visible_lane_for_test(
        lane: &EventLane,
        installed: &mut EventsSubscription,
        events: &mut async_nats::Subscriber,
        visible: Option<&str>,
        desktop_id: &str,
    ) {
        apply_visible_lane(lane, installed, events, visible, desktop_id).await
    }

    /// The queue ceiling, so a test can overflow it without duplicating the
    /// number (and so a change to the constant moves the test with it).
    pub(crate) const BACKLOG_CAPACITY: usize = EVENT_BACKLOG_CAPACITY;
}
