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
use std::{collections::HashMap, sync::LazyLock};

#[derive(Default)]
struct FailureEpisodeState {
    category: String,
    started_at: u64,
    attempts: u64,
    reports: u8,
    last_error: String,
}

#[derive(Default)]
struct FailureEpisode(Mutex<FailureEpisodeState>);

#[derive(Default)]
struct LogQuota {
    window_started_at: u64,
    emitted: u8,
}

/// This is intentionally process-wide, rather than an episode field. A
/// broken broker can alternate network/auth/task symptoms and would otherwise
/// reset each individual episode's counter forever. Support logs remain useful
/// without letting a 24-hour outage fill the disk.
static FAILURE_LOG_QUOTAS: LazyLock<Mutex<HashMap<String, LogQuota>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const FAILURE_LOG_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_FAILURE_LOGS_PER_CATEGORY: u8 = 16;

fn permit_failure_log(category: &str) -> bool {
    let now = unix_millis();
    let mut quotas = FAILURE_LOG_QUOTAS.lock().unwrap();
    let quota = quotas.entry(category.to_string()).or_default();
    if now.saturating_sub(quota.window_started_at) >= FAILURE_LOG_WINDOW_MS {
        quota.window_started_at = now;
        quota.emitted = 0;
    }
    if quota.emitted >= MAX_FAILURE_LOGS_PER_CATEGORY {
        return false;
    }
    quota.emitted += 1;
    true
}

impl FailureEpisode {
    fn record(&self, category: &str, error: impl std::fmt::Display) -> Option<String> {
        let mut episode = self.0.lock().unwrap();
        let error = error.to_string();
        if episode.category != category {
            episode.category = category.to_string();
            episode.started_at = unix_millis();
            episode.attempts = 1;
            episode.reports = 0;
            episode.last_error = error;
            if permit_failure_log(category) {
                episode.reports = 1;
                return Some(format!(
                    "remote: {category} failure episode started [{}]: {}",
                    support_code_for_category(category),
                    episode.last_error
                ));
            }
            return None;
        }
        episode.attempts = episode.attempts.saturating_add(1);
        episode.last_error = error;
        if episode.reports < 15
            && episode.attempts.is_power_of_two()
            && permit_failure_log(category)
        {
            episode.reports += 1;
            return Some(format!(
                "remote: {category} failure persists [{}] (attempt {}): {}",
                support_code_for_category(category),
                episode.attempts,
                episode.last_error
            ));
        }
        None
    }

    fn recovered(&self) -> Option<String> {
        let mut episode = self.0.lock().unwrap();
        if episode.category.is_empty() {
            return None;
        }
        let line = permit_failure_log(&episode.category).then(|| {
            format!(
                "remote: {} failure recovered [{}] after {} attempts and {}ms",
                episode.category,
                support_code_for_category(&episode.category),
                episode.attempts,
                unix_millis().saturating_sub(episode.started_at)
            )
        });
        *episode = FailureEpisodeState::default();
        line
    }
}

fn support_code_for_category(category: &str) -> &'static str {
    match category {
        "network" | "credential_network" => "NW001",
        "remote_server" => "SV001",
        "service_authorization" => "AU001",
        "credential_expired" | "credential_connect" => "AU002",
        "slow_consumer" => "RT002",
        "command_subscription" => "RT003",
        "transfer_subscription" => "RT004",
        "event_publish" => "RT005",
        "heartbeat_publish" | "state_publish" => "RT006",
        "web_bind" => "LC002",
        "revoked" => "PA001",
        "local" => "LC001",
        _ => "LC999",
    }
}

static EVENT_PUBLISH_EPISODE: FailureEpisode = FailureEpisode(Mutex::new(FailureEpisodeState {
    category: String::new(),
    started_at: 0,
    attempts: 0,
    reports: 0,
    last_error: String::new(),
}));
static START_EPISODE: FailureEpisode = FailureEpisode(Mutex::new(FailureEpisodeState {
    category: String::new(),
    started_at: 0,
    attempts: 0,
    reports: 0,
    last_error: String::new(),
}));
static CREDENTIAL_EPISODE: FailureEpisode = FailureEpisode(Mutex::new(FailureEpisodeState {
    category: String::new(),
    started_at: 0,
    attempts: 0,
    reports: 0,
    last_error: String::new(),
}));
static HEARTBEAT_PUBLISH_EPISODE: FailureEpisode =
    FailureEpisode(Mutex::new(FailureEpisodeState {
        category: String::new(),
        started_at: 0,
        attempts: 0,
        reports: 0,
        last_error: String::new(),
    }));

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
    /// Number of emitted lines in this episode. Capped so a prolonged outage
    /// can never fill the console or a redirected log file.
    reports: AtomicU8,
}

impl DropCounters {
    const fn new() -> Self {
        Self {
            dropping: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            reports: AtomicU8::new(0),
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
        let _ = now;
        if first {
            self.reports.store(1, Ordering::Relaxed);
            Some(format!(
                "remote: {why}; dropping {event_type} for {session_id} (backfill on reconnect heals the gap)"
            ))
        } else if total.is_power_of_two() && self.reports.load(Ordering::Relaxed) < 15 {
            self.reports.fetch_add(1, Ordering::Relaxed);
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
            self.reports.store(0, Ordering::Relaxed);
            Some(format!(
                "remote: event publish recovered; dropped {dropped} events during the backlog"
            ))
        } else {
            None
        }
    }
}

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

/// Generation-local health reported by async-nats. Keeping this beside the
/// client prevents a late callback from an old credential generation from
/// poisoning the currently active bridge.
#[derive(Default)]
struct NatsHealth {
    reconnect_required: AtomicBool,
    service_config_error: AtomicBool,
    /// This belongs to one NATS generation, never the process. A late event
    /// from an old socket must not decide whether a newer JWT is terminal.
    credential_was_refreshed: AtomicBool,
    authorization_rejection_logged: AtomicBool,
    event_episode: FailureEpisode,
}

impl NatsHealth {
    fn handle_event(&self, event: &async_nats::Event) {
        use async_nats::{ClientError, Event, ServerError};
        match event {
            Event::Connected => {
                self.reconnect_required.store(false, Ordering::Release);
                self.authorization_rejection_logged
                    .store(false, Ordering::Release);
                if let Some(line) = self.event_episode.recovered() {
                    eprintln!("{line}");
                }
            }
            Event::Disconnected => {
                if let Some(line) = self.event_episode.record("network", "NATS disconnected") {
                    eprintln!("{line}");
                }
            }
            Event::ServerError(ServerError::AuthorizationViolation) => {
                self.mark_authorization_rejected();
            }
            // async-nats reports an authorization rejection that occurs while
            // reconnecting as `ClientError::Other`, not `ServerError`. Treat
            // this transient-client shape as a failed generation: the runtime
            // supervisor replaces it with a fresh short-lived bridge JWT.
            Event::ClientError(ClientError::Other(error)) if is_authorization_violation(error) => {
                self.mark_authorization_rejected();
            }
            Event::ClientError(ClientError::MaxReconnects) => {
                self.reconnect_required.store(true, Ordering::Release);
                if let Some(line) = self
                    .event_episode
                    .record("network", "NATS reconnect budget exhausted")
                {
                    eprintln!("{line}");
                }
            }
            Event::Closed => {
                self.reconnect_required.store(true, Ordering::Release);
                if let Some(line) = self
                    .event_episode
                    .record("network", "NATS connection closed")
                {
                    eprintln!("{line}");
                }
            }
            Event::SlowConsumer(subscription) => {
                if let Some(line) = self.event_episode.record("slow_consumer", subscription) {
                    eprintln!("{line}");
                }
            }
            Event::ServerError(ServerError::SlowConsumer(subscription)) => {
                if let Some(line) = self.event_episode.record("slow_consumer", subscription) {
                    eprintln!("{line}");
                }
            }
            Event::ServerError(error) => {
                if let Some(line) = self.event_episode.record("remote_server", error) {
                    eprintln!("{line}");
                }
            }
            Event::ClientError(error) => {
                if let Some(line) = self.event_episode.record("network", error) {
                    eprintln!("{line}");
                }
            }
            _ => {}
        }
    }

    fn needs_reconnect(&self) -> bool {
        self.reconnect_required.load(Ordering::Acquire)
    }

    fn is_terminal(&self) -> bool {
        self.service_config_error.load(Ordering::Acquire)
    }

    fn mark_authorization_rejected(&self) {
        // The reconnect loop can deliver the same error indefinitely. One
        // state transition is enough to make the supervisor refresh the JWT;
        // suppress the duplicate events so a sleeping laptop cannot flood its
        // console while that handoff is in progress.
        if self.credential_was_refreshed.load(Ordering::Acquire) {
            self.service_config_error.store(true, Ordering::Release);
            if !self
                .authorization_rejection_logged
                .swap(true, Ordering::AcqRel)
            {
                eprintln!(
                    "remote: freshly refreshed NATS credential was rejected [AU001]; entering service_authorization"
                );
            }
            return;
        }
        self.reconnect_required.store(true, Ordering::Release);
        if !self
            .authorization_rejection_logged
            .swap(true, Ordering::AcqRel)
        {
            eprintln!("remote: NATS authorization rejected [AU002]; refreshing bridge credentials");
        }
    }
}

fn is_authorization_violation(error: &str) -> bool {
    error.trim().eq_ignore_ascii_case("authorization violation")
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn reconnect_delay(attempt: usize, random: f64) -> std::time::Duration {
    let exponent = attempt.min(5) as u32;
    let base_secs = (1u64 << exponent).min(30);
    std::time::Duration::from_secs_f64((base_secs as f64 * (0.8 + random * 0.4)).min(30.0))
}

struct ConnectedNats {
    client: async_nats::Client,
    health: Arc<NatsHealth>,
}

/// Active remote connection. Holds async-nats client + command/event tasks;
/// on stop, aborts the tasks and drops the client.
struct RemoteState {
    generation_id: u64,
    /// Raw client, kept to derive real connection state for [`status`].
    client: async_nats::Client,
    nats_health: Arc<NatsHealth>,
    nats_url: String,
    pair_id: String,
    desktop_id: String,
    desktop_public_key: String,
    bridge_instance_id: String,
    /// Ordered event queue → single drain task per connection. The drain holds
    /// a clone of the client so the connection stays alive while events are in
    /// flight.
    event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    drop_counters: Arc<DropCounters>,
    event_task: tokio::task::JoinHandle<()>,
    cmd_task: tokio::task::JoinHandle<()>,
    transfer_task: tokio::task::JoinHandle<()>,
    heartbeat_task: tokio::task::JoinHandle<()>,
    refresh_task: tokio::task::JoinHandle<()>,
    /// `None` outside the test environment, or when the optional test web
    /// server failed to bind. The phone bridge remains available either way.
    web_task: Option<tokio::task::JoinHandle<()>>,
    /// Test-only web client URL for THIS machine; `None` outside the test
    /// environment or when bind failed.
    web_url: Option<String>,
    /// Test-only web client URL a phone on the same LAN can reach; `None`
    /// outside the test environment, when bind failed, or without a LAN route.
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

/// State whose correctness spans credential and transport generations. A
/// generation swap must never clear command single-flight replies or pairing
/// confirmation, otherwise a retried command can execute twice and a paired
/// phone can be forced through an unnecessary claim flow.
#[derive(Clone)]
struct BridgeRuntimeShared {
    pair_id: String,
    reply_slots: commands::ReplySlots,
    pairing_confirmed: Arc<AtomicBool>,
    bridge_instance_id: String,
    drop_counters: Arc<DropCounters>,
    next_generation_id: Arc<AtomicU64>,
}

static BRIDGE_SHARED: Mutex<Option<BridgeRuntimeShared>> = Mutex::new(None);

fn shared_runtime(
    pair_id: &str,
    pairing_confirmed: bool,
    rotate_epoch: bool,
) -> BridgeRuntimeShared {
    let mut guard = BRIDGE_SHARED.lock().unwrap();
    if let Some(shared) = guard.as_mut().filter(|shared| shared.pair_id == pair_id) {
        if rotate_epoch {
            shared.bridge_instance_id =
                format!("bridge_{}", nkeys::KeyPair::new_user().public_key());
        }
        if pairing_confirmed {
            shared.pairing_confirmed.store(true, Ordering::Release);
        }
        return shared.clone();
    }
    let shared = BridgeRuntimeShared {
        pair_id: pair_id.to_string(),
        reply_slots: commands::new_reply_slots(),
        pairing_confirmed: Arc::new(AtomicBool::new(pairing_confirmed)),
        bridge_instance_id: format!("bridge_{}", nkeys::KeyPair::new_user().public_key()),
        drop_counters: Arc::new(DropCounters::new()),
        next_generation_id: Arc::new(AtomicU64::new(1)),
    };
    *guard = Some(shared.clone());
    shared
}

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
static START_RETRY_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static START_RETRY_SINCE: AtomicU64 = AtomicU64::new(0);
static START_RETRY_NEXT_AT: AtomicU64 = AtomicU64::new(0);
static CREDENTIAL_REFRESHING: AtomicBool = AtomicBool::new(false);
/// A finished critical task is rebuilt automatically. Bound repeated
/// reconnects so a deterministic panic cannot spin forever.
static RUNTIME_RECONNECT_RUNNING: AtomicBool = AtomicBool::new(false);
static RUNTIME_RECONNECT_ATTEMPTS: AtomicU8 = AtomicU8::new(0);
static RUNTIME_FAILURE_WINDOW_STARTED: AtomicU64 = AtomicU64::new(0);
const MAX_RUNTIME_RECONNECT_ATTEMPTS: u8 = 3;
const RUNTIME_FAILURE_WINDOW_MS: u64 = 10 * 60 * 1_000;
/// Do not forgive a crash-loop merely because a replacement generation stayed
/// alive for one poll. Only a sustained healthy minute resets the budget.
#[cfg(not(test))]
const RUNTIME_HEALTHY_RESET_SECS: u8 = 60;
static WEB_RECONNECT_RUNNING: AtomicBool = AtomicBool::new(false);
static WEB_RECONNECT_ATTEMPTS: AtomicU8 = AtomicU8::new(0);
const MAX_WEB_RECONNECT_ATTEMPTS: u8 = 3;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemotePhase {
    Stopped,
    Connecting,
    Ready,
    Reconnecting,
    Refreshing,
    Failed,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFailureReason {
    Network,
    SystemSleep,
    CredentialExpired,
    CredentialRevoked,
    ServiceAuthorization,
    RemoteServer,
    Protocol,
    GenerationUnhealthy,
    Local,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryProgress {
    pub attempt: u64,
    pub max_attempts: Option<u64>,
    pub since: u64,
    pub next_retry_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub phase: RemotePhase,
    pub reason: Option<RemoteFailureReason>,
    pub recovery: Option<RecoveryProgress>,
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
    /// Test-only web client URL for this machine; `None` outside the test
    /// environment or if the web server failed to bind.
    pub web_url: Option<String>,
    /// Test-only web client URL a phone on the same LAN can reach; `None` if
    /// unavailable or outside the test environment.
    pub web_lan_url: Option<String>,
    /// Non-critical local web listener failure. It never changes the main
    /// Remote phase or readiness.
    pub warning_code: Option<String>,
}

fn retryable_start_status(status: &RemoteStatus) -> bool {
    matches!(status.phase, RemotePhase::Reconnecting)
        && matches!(
            status.reason,
            Some(RemoteFailureReason::Network | RemoteFailureReason::RemoteServer)
        )
}

fn runtime_active(status: &RemoteStatus) -> bool {
    matches!(
        status.phase,
        RemotePhase::Connecting
            | RemotePhase::Ready
            | RemotePhase::Reconnecting
            | RemotePhase::Refreshing
    )
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        phase: RemotePhase::Stopped,
        reason: None,
        recovery: None,
        nats_url: String::new(),
        pair_id: String::new(),
        pairing_code: None,
        pairing_code_expires_at: None,
        desktop_id: String::new(),
        desktop_public_key: String::new(),
        web_url: None,
        web_lan_url: None,
        warning_code: None,
    }
}

pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    // An explicit user reconnect gets fresh automatic-reconnect budgets.
    RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    RUNTIME_FAILURE_WINDOW_STARTED.store(0, Ordering::Release);
    WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    CREDENTIAL_REFRESHING.store(false, Ordering::Release);
    START_RETRY_ATTEMPTS.store(0, Ordering::Release);
    START_RETRY_SINCE.store(0, Ordering::Release);
    START_RETRY_NEXT_AT.store(0, Ordering::Release);
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
        if runtime_active(&current) {
            return Ok(current);
        }
    }
    let replacing_generation = STATE.lock().unwrap().is_some();
    *LAST_ERROR_CODE.lock().unwrap() = Some("connecting".to_string());

    // A remote/server failure here (offline, revoked, HTTP error) is not a
    // program fault — surface it as a localized, not-running status instead of
    // throwing a raw transport string at the UI. Local failures (NKey, disk)
    // keep propagating as `Err`.
    let (creds, pairing_code, pairing_code_expires_at) = match establish().await {
        Ok(value) => value,
        Err(error) => {
            return start_failure(error);
        }
    };
    if !START_REQUESTED.load(Ordering::Acquire) {
        return Ok(empty());
    }
    let connected_nats = match connect_nats(&creds, false).await {
        Ok(connection) => connection,
        Err(error) => {
            return start_failure(error);
        }
    };
    let client = connected_nats.client;
    let nats_health = connected_nats.health;
    if !START_REQUESTED.load(Ordering::Acquire) {
        return Ok(empty());
    }
    let shared = shared_runtime(&creds.pair_id, pairing_code.is_none(), replacing_generation);
    let pairing_confirmed = shared.pairing_confirmed.clone();
    if pairing_confirmed.load(Ordering::Acquire) {
        pairing::save_creds(&creds)?;
    }
    let desktop_public_key = pairing::public_key(&creds)?;
    let bridge_instance_id = shared.bridge_instance_id.clone();
    let generation_id = shared.next_generation_id.fetch_add(1, Ordering::AcqRel);
    let pair_id = creds.pair_id.clone();

    // Command-id dedup cache lives OUTSIDE the command loop: credential
    // refresh swaps the loop every JWT TTL, and a cache tied to the loop would
    // be wiped each swap — retrying clients would re-execute commands (a
    // retried prompt = a duplicated user message + run).
    let reply_slots = shared.reply_slots.clone();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
    let handshake_state = commands::HandshakeState::new(
        creds.clone(),
        pairing_confirmed.clone(),
        bridge_instance_id.clone(),
    );

    let (command_ready_tx, command_ready_rx) = tokio::sync::oneshot::channel();
    let cmd_task = tokio::spawn(commands::command_loop_with_ready(
        client.clone(),
        pair_id.clone(),
        reply_slots.clone(),
        handshake_state.clone(),
        Some(command_ready_tx),
    ));
    let (transfer_ready_tx, transfer_ready_rx) = tokio::sync::oneshot::channel();
    let transfer_task = transfer::spawn_transfer_loop_with_ready(
        client.clone(),
        pair_id.clone(),
        handshake_state.active_flag(),
        Some(transfer_ready_tx),
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
    // Readiness barrier: give the subscription tasks a scheduling turn, then
    // publish and flush the first presence packet. The generation is not
    // installed in STATE (and therefore cannot report ready) until the broker
    // has acknowledged every command queued before this flush.
    tokio::task::yield_now().await;
    let readiness = async {
        command_ready_rx.await.map_err(|_| {
            crate::AppError::Message("Remote command subscription stopped during readiness".into())
        })?;
        transfer_ready_rx.await.map_err(|_| {
            crate::AppError::Message("Remote transfer subscription stopped during readiness".into())
        })?;
        let payload = serde_json::to_vec(&light_presence_payload(&pair_id, &bridge_instance_id))?;
        client
            .publish(format!("p.{pair_id}.presence"), payload.into())
            .await
            .map_err(|error| crate::AppError::RemoteTransport(error.to_string()))?;
        client
            .flush()
            .await
            .map_err(|error| crate::AppError::RemoteTransport(error.to_string()))
    };
    let readiness = tokio::time::timeout(std::time::Duration::from_secs(10), readiness)
        .await
        .map_err(|_| crate::AppError::RemoteTransport("Remote readiness timed out".into()))
        .and_then(|result| result);
    if let Err(error) = readiness {
        event_task.abort();
        cmd_task.abort();
        transfer_task.abort();
        heartbeat_task.abort();
        refresh_task.abort();
        return start_failure(error);
    }
    // The browser client is a test-environment-only validation surface. Keep
    // the NATS/mobile bridge available in every environment, but never expose
    // the unauthenticated local HTTP listener in production or custom envs.
    let web_enabled = web_client_enabled();
    let (web_task, web_url, web_lan_url) = if web_enabled {
        match bind_web_listener().await {
            Ok(listener) => (
                Some(spawn_web_server(listener)),
                Some(format!("http://localhost:{WEB_PORT}")),
                lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}")),
            ),
            Err(error) => {
                eprintln!("remote: web client bind failed [LC002]: {error}");
                (None, None, None)
            }
        }
    } else {
        (None, None, None)
    };
    // A failed web bind is non-fatal (the bridge still runs) and is retried
    // silently before the UI is asked to intervene.
    let web_bind_failed = web_enabled && web_task.is_none();

    let status = RemoteStatus {
        phase: RemotePhase::Ready,
        reason: None,
        recovery: None,
        nats_url: creds.nats_url.clone(),
        pair_id: pair_id.clone(),
        pairing_code: pairing_code.clone(),
        pairing_code_expires_at,
        desktop_id: creds.desktop_id.clone(),
        desktop_public_key: desktop_public_key.clone(),
        web_url: web_url.clone(),
        web_lan_url: web_lan_url.clone(),
        warning_code: web_bind_failed.then(|| "web_bind".to_string()),
    };
    // Keep the previous generation serving until every readiness prerequisite
    // above is complete. Replacing the pointer is the hand-off point; only
    // after it do we cancel the old generation.
    let previous = STATE.lock().unwrap().replace(RemoteState {
        generation_id,
        client,
        nats_health,
        nats_url: creds.nats_url,
        pair_id,
        desktop_id: creds.desktop_id,
        desktop_public_key,
        bridge_instance_id: bridge_instance_id.clone(),
        event_tx,
        drop_counters: shared.drop_counters.clone(),
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
    if let Some(previous) = previous {
        abort_generation(previous);
    }
    *LAST_ERROR_CODE.lock().unwrap() = None;
    if let Some(line) = START_EPISODE.recovered() {
        eprintln!("{line}");
    }
    START_RETRY_ATTEMPTS.store(0, Ordering::Release);
    START_RETRY_SINCE.store(0, Ordering::Release);
    START_RETRY_NEXT_AT.store(0, Ordering::Release);
    spawn_runtime_supervisor(bridge_instance_id, generation_id);
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
            START_RETRY_SINCE
                .compare_exchange(0, unix_millis(), Ordering::AcqRel, Ordering::Acquire)
                .ok();
            loop {
                let jitter = 0.8 + rand::random::<f64>() * 0.4;
                let jittered =
                    std::time::Duration::from_secs_f64((delay.as_secs_f64() * jitter).min(30.0));
                START_RETRY_NEXT_AT.store(
                    unix_millis().saturating_add(jittered.as_millis() as u64),
                    Ordering::Release,
                );
                tokio::time::sleep(jittered).await;
                START_RETRY_ATTEMPTS.fetch_add(1, Ordering::AcqRel);
                if !START_REQUESTED.load(Ordering::Acquire)
                    || matches!(status().phase, RemotePhase::Ready)
                {
                    break;
                }
                match start_once(false).await {
                    Ok(status) if matches!(status.phase, RemotePhase::Ready) => break,
                    Ok(status) if retryable_start_status(&status) => {
                        delay = delay
                            .saturating_mul(2)
                            .min(std::time::Duration::from_secs(30));
                    }
                    Ok(_) | Err(_) => break,
                }
            }
            START_RETRY_NEXT_AT.store(0, Ordering::Release);
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
#[cfg_attr(test, allow(dead_code))]
fn spawn_runtime_reconnect() {
    #[cfg(test)]
    return;

    #[cfg(not(test))]
    {
        if RUNTIME_RECONNECT_RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        let attempt = record_runtime_failure(unix_millis());
        eprintln!(
            "remote: critical task stopped [RT001]; rebuilding generation ({attempt}/{MAX_RUNTIME_RECONNECT_ATTEMPTS})"
        );
        tauri::async_runtime::spawn(async move {
            let result = start_once(true).await;
            match &result {
                Ok(status) if retryable_start_status(status) => spawn_start_retry(),
                Err(error) => {
                    eprintln!("remote: automatic bridge reconnect failed [RT001]: {error}");
                    *LAST_ERROR_CODE.lock().unwrap() = Some("reconnect_required".to_string());
                }
                _ => {}
            }
            RUNTIME_RECONNECT_RUNNING.store(false, Ordering::Release);
        });
    }
}

fn record_runtime_failure(now: u64) -> u8 {
    let started = RUNTIME_FAILURE_WINDOW_STARTED.load(Ordering::Acquire);
    if started == 0 || now.saturating_sub(started) > RUNTIME_FAILURE_WINDOW_MS {
        RUNTIME_FAILURE_WINDOW_STARTED.store(now, Ordering::Release);
        RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    }
    RUNTIME_RECONNECT_ATTEMPTS.fetch_add(1, Ordering::AcqRel) + 1
}

/// Watch the active bridge from the process runtime, independently of any UI
/// status polling. This is essential for Remote clients: a phone must recover
/// even when the desktop window is hidden or no frontend has mounted yet.
#[cfg(not(test))]
struct RemoteSupervisor {
    bridge_instance_id: String,
    generation_id: u64,
}

#[cfg(not(test))]
impl RemoteSupervisor {
    async fn run(self) {
        let bridge_instance_id = self.bridge_instance_id;
        let generation_id = self.generation_id;
        let mut healthy_secs = 0u8;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !START_REQUESTED.load(Ordering::Acquire) {
                return;
            }
            let unhealthy = {
                let guard = STATE.lock().unwrap();
                let Some(state) = guard.as_ref().filter(|state| {
                    state.bridge_instance_id == bridge_instance_id
                        && state.generation_id == generation_id
                }) else {
                    return;
                };
                if state.nats_health.is_terminal() {
                    return;
                }
                state.nats_health.needs_reconnect()
                    || state.cmd_task.is_finished()
                    || state.transfer_task.is_finished()
                    || state.event_task.is_finished()
                    || state.heartbeat_task.is_finished()
                    || state.refresh_task.is_finished()
            };
            if !unhealthy {
                healthy_secs = healthy_secs.saturating_add(1);
                if healthy_secs >= RUNTIME_HEALTHY_RESET_SECS {
                    RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
                    RUNTIME_FAILURE_WINDOW_STARTED.store(0, Ordering::Release);
                }
                continue;
            }
            if RUNTIME_RECONNECT_RUNNING.load(Ordering::Acquire) {
                continue;
            }
            if RUNTIME_RECONNECT_ATTEMPTS.load(Ordering::Acquire) >= MAX_RUNTIME_RECONNECT_ATTEMPTS
            {
                *LAST_ERROR_CODE.lock().unwrap() = Some("reconnect_required".to_string());
                return;
            }
            spawn_runtime_reconnect();
            return;
        }
    }
}

fn spawn_runtime_supervisor(bridge_instance_id: String, generation_id: u64) {
    #[cfg(test)]
    {
        let _ = (bridge_instance_id, generation_id);
    }

    #[cfg(not(test))]
    tauri::async_runtime::spawn(
        RemoteSupervisor {
            bridge_instance_id,
            generation_id,
        }
        .run(),
    );
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
                        eprintln!("remote: local web listener reconnected [LC002]");
                    } else {
                        web_task.abort();
                    }
                }
                Err(error) => {
                    eprintln!("remote: local web listener reconnect failed [LC002]: {error}")
                }
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
            eprintln!("remote: replacing legacy pairing [PA003]");
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
                eprintln!("remote: persisted pairing was revoked [PA001]; creating a new pairing");
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
            if let Some(line) = START_EPISODE.record(code, &error) {
                eprintln!("{line}");
            }
            *LAST_ERROR_CODE.lock().unwrap() = Some(code.to_string());
            Ok(RemoteStatus {
                phase: match code {
                    "network" | "server" => RemotePhase::Reconnecting,
                    "revoked" => RemotePhase::Revoked,
                    _ => RemotePhase::Failed,
                },
                reason: Some(match code {
                    "network" => RemoteFailureReason::Network,
                    "revoked" => RemoteFailureReason::CredentialRevoked,
                    "service_authorization" => RemoteFailureReason::ServiceAuthorization,
                    "server" => RemoteFailureReason::RemoteServer,
                    _ => RemoteFailureReason::Local,
                }),
                recovery: matches!(code, "network" | "server").then(|| RecoveryProgress {
                    attempt: START_RETRY_ATTEMPTS.load(Ordering::Acquire),
                    max_attempts: None,
                    since: START_RETRY_SINCE.load(Ordering::Acquire),
                    next_retry_at: None,
                }),
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
    credential_was_refreshed: bool,
) -> Result<ConnectedNats, crate::AppError> {
    let key_pair = std::sync::Arc::new(
        nkeys::KeyPair::from_seed(&creds.nkey_seed)
            .map_err(|error| crate::AppError::Message(format!("Invalid desktop NKey: {error}")))?,
    );
    let health = Arc::new(NatsHealth::default());
    health
        .credential_was_refreshed
        .store(credential_was_refreshed, Ordering::Release);
    let event_health = health.clone();
    let options = async_nats::ConnectOptions::with_jwt(creds.user_jwt.clone(), move |nonce| {
        let key_pair = key_pair.clone();
        async move { key_pair.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .custom_inbox_prefix(format!("p.{}.rep.{}", creds.pair_id, creds.desktop_id))
    .reconnect_delay_callback(|attempt| reconnect_delay(attempt, rand::random::<f64>()))
    .event_callback(move |event| {
        let health = event_health.clone();
        async move { health.handle_event(&event) }
    });
    let client = options.connect(&creds.nats_url).await.map_err(|error| {
        let message = format!("Failed to connect to NATS: {error}");
        classify_nats_connect_error(error.kind(), message)
    })?;
    Ok(ConnectedNats { client, health })
}

fn classify_nats_connect_error(
    kind: async_nats::ConnectErrorKind,
    message: String,
) -> crate::AppError {
    match kind {
        async_nats::ConnectErrorKind::Authentication
        | async_nats::ConnectErrorKind::AuthorizationViolation => {
            crate::AppError::RemoteAuthorization(message)
        }
        async_nats::ConnectErrorKind::Dns
        | async_nats::ConnectErrorKind::TimedOut
        | async_nats::ConnectErrorKind::Io
        | async_nats::ConnectErrorKind::Tls
        | async_nats::ConnectErrorKind::MaxReconnects => crate::AppError::RemoteTransport(message),
        async_nats::ConnectErrorKind::ServerParse => crate::AppError::Message(message),
    }
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
    transfer::clear_preview_cache();
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
    RUNTIME_FAILURE_WINDOW_STARTED.store(0, Ordering::Release);
    WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
    let status = stop_runtime();
    *BRIDGE_SHARED.lock().unwrap() = None;
    status
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

/// Platform adapters report only these facts. Keeping their policy pure makes
/// suspend/resume behavior testable without macOS/Windows/Linux notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerEvent {
    Suspend,
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PowerTransition {
    stop_generation: bool,
    start_generation: bool,
    rotate_epoch: bool,
}

fn power_transition(event: PowerEvent, desired_running: bool) -> PowerTransition {
    match (event, desired_running) {
        (PowerEvent::Suspend, true) => PowerTransition {
            stop_generation: true,
            start_generation: false,
            rotate_epoch: false,
        },
        (PowerEvent::Resume, true) => PowerTransition {
            stop_generation: false,
            start_generation: true,
            rotate_epoch: true,
        },
        _ => PowerTransition {
            stop_generation: false,
            start_generation: false,
            rotate_epoch: false,
        },
    }
}

/// OS power lifecycle adapter. Suspend tears down only generation-local
/// resources; pairing, reply deduplication, drop episodes, and desired-running
/// intent remain available for resume.
pub async fn handle_system_suspend() {
    notify_mobile_disconnect("system_sleep").await;
    let transition = power_transition(PowerEvent::Suspend, START_REQUESTED.load(Ordering::Acquire));
    if transition.stop_generation {
        *LAST_ERROR_CODE.lock().unwrap() = Some("system_sleep".to_string());
        CREDENTIAL_REFRESHING.store(false, Ordering::Release);
        let _ = stop_runtime();
    }
}

/// Resume with a fresh bridge epoch so the phone cannot mistake a half-open
/// pre-sleep socket for the current generation. `establish()` refreshes the
/// JWT before the NATS connection is built.
pub fn handle_system_resume() {
    let transition = power_transition(PowerEvent::Resume, START_REQUESTED.load(Ordering::Acquire));
    if !transition.start_generation {
        return;
    }
    if transition.rotate_epoch {
        if let Some(shared) = BRIDGE_SHARED.lock().unwrap().as_mut() {
            shared.bridge_instance_id =
                format!("bridge_{}", nkeys::KeyPair::new_user().public_key());
        }
    }
    *LAST_ERROR_CODE.lock().unwrap() = Some("system_sleep".to_string());
    tauri::async_runtime::spawn(async {
        match start_once(false).await {
            Ok(status) if retryable_start_status(&status) => spawn_start_retry(),
            Ok(_) => {}
            Err(error) => {
                eprintln!("remote: resume recovery failed [PW001]: {error}");
                *LAST_ERROR_CODE.lock().unwrap() = Some("reconnect_required".to_string());
            }
        }
    });
}

fn abort_generation(state: RemoteState) {
    debug_assert!(state.generation_id > 0, "generation IDs start at one");
    state.event_task.abort();
    state.cmd_task.abort();
    state.transfer_task.abort();
    state.heartbeat_task.abort();
    state.refresh_task.abort();
    if let Some(web_task) = state.web_task {
        web_task.abort();
    }
    // In-flight transfers are generation-scoped. Preview artifacts are kept.
    transfer::clear_transfers();
}

fn stop_runtime() -> RemoteStatus {
    if let Some(state) = STATE.lock().unwrap().take() {
        let pair_id = state.pair_id.clone();
        let client = state.client.clone();
        let bridge_instance_id = state.bridge_instance_id.clone();
        tauri::async_runtime::spawn(async move {
            let subject = format!("p.{pair_id}.presence");
            let payload = serde_json::to_vec(&json!({
                "online": false,
                "pairId": pair_id,
                "bridgeInstanceId": bridge_instance_id,
                "lastHeartbeatTs": unix_timestamp(),
            }))
            .unwrap_or_default();
            let _ = client.publish(subject, payload.into()).await;
            let _ = client.flush().await;
        });
        abort_generation(state);
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
            let terminal_service_error = s.nats_health.is_terminal();
            let generation_unhealthy = critical_task_dead || s.nats_health.needs_reconnect();
            let nats_reconnecting =
                s.client.connection_state() != async_nats::connection::State::Connected;
            let reconnect_in_flight = RUNTIME_RECONNECT_RUNNING.load(Ordering::Acquire);
            let can_auto_reconnect =
                RUNTIME_RECONNECT_ATTEMPTS.load(Ordering::Acquire) < MAX_RUNTIME_RECONNECT_ATTEMPTS;
            let reconnecting = (generation_unhealthy || nats_reconnecting)
                && !terminal_service_error
                && START_REQUESTED.load(Ordering::Acquire)
                && (!generation_unhealthy || reconnect_in_flight || can_auto_reconnect);
            let connected = !generation_unhealthy && !terminal_service_error && !nats_reconnecting;

            // The local Web listener is test-only and retries independently so
            // a busy port never disrupts the healthy phone bridge.
            let web_enabled = web_client_enabled();
            let web_dead = web_enabled && s.web_task.as_ref().is_none_or(|task| task.is_finished());
            if web_enabled && !web_dead {
                WEB_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
            }
            let web_reconnect_in_flight =
                web_enabled && WEB_RECONNECT_RUNNING.load(Ordering::Acquire);
            let can_reconnect_web = web_enabled
                && WEB_RECONNECT_ATTEMPTS.load(Ordering::Acquire) < MAX_WEB_RECONNECT_ATTEMPTS;
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
                phase: if terminal_service_error {
                    RemotePhase::Failed
                } else if CREDENTIAL_REFRESHING.load(Ordering::Acquire) {
                    RemotePhase::Refreshing
                } else if connected {
                    RemotePhase::Ready
                } else if reconnecting {
                    RemotePhase::Reconnecting
                } else {
                    RemotePhase::Failed
                },
                reason: if terminal_service_error {
                    Some(RemoteFailureReason::ServiceAuthorization)
                } else if CREDENTIAL_REFRESHING.load(Ordering::Acquire) {
                    Some(RemoteFailureReason::CredentialExpired)
                } else if generation_unhealthy {
                    Some(RemoteFailureReason::GenerationUnhealthy)
                } else if nats_reconnecting {
                    Some(RemoteFailureReason::Network)
                } else {
                    None
                },
                recovery: reconnecting.then(|| RecoveryProgress {
                    attempt: if generation_unhealthy {
                        RUNTIME_RECONNECT_ATTEMPTS.load(Ordering::Acquire) as u64
                    } else {
                        START_RETRY_ATTEMPTS.load(Ordering::Acquire)
                    },
                    max_attempts: generation_unhealthy
                        .then_some(MAX_RUNTIME_RECONNECT_ATTEMPTS as u64),
                    since: if generation_unhealthy {
                        RUNTIME_FAILURE_WINDOW_STARTED.load(Ordering::Acquire)
                    } else {
                        START_RETRY_SINCE.load(Ordering::Acquire)
                    },
                    next_retry_at: match START_RETRY_NEXT_AT.load(Ordering::Acquire) {
                        0 => None,
                        value => Some(value),
                    },
                }),
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                pairing_code,
                pairing_code_expires_at,
                desktop_id: s.desktop_id.clone(),
                desktop_public_key: s.desktop_public_key.clone(),
                web_url: s.web_url.clone(),
                web_lan_url: s.web_lan_url.clone(),
                warning_code: web_reconnect_exhausted.then(|| "web_bind".to_string()),
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
            let (phase, reason) = if reconnecting {
                (
                    RemotePhase::Reconnecting,
                    Some(match error_code.as_deref() {
                        Some("server") => RemoteFailureReason::RemoteServer,
                        _ => RemoteFailureReason::Network,
                    }),
                )
            } else {
                match error_code.as_deref() {
                    Some("revoked") => (
                        RemotePhase::Revoked,
                        Some(RemoteFailureReason::CredentialRevoked),
                    ),
                    Some("service_authorization" | "service_config") => (
                        RemotePhase::Failed,
                        Some(RemoteFailureReason::ServiceAuthorization),
                    ),
                    Some("reconnect_required") => (
                        RemotePhase::Failed,
                        Some(RemoteFailureReason::GenerationUnhealthy),
                    ),
                    Some("system_sleep") => (
                        RemotePhase::Reconnecting,
                        Some(RemoteFailureReason::SystemSleep),
                    ),
                    Some("connecting") => (RemotePhase::Connecting, None),
                    Some("credential_expired") => (
                        RemotePhase::Refreshing,
                        Some(RemoteFailureReason::CredentialExpired),
                    ),
                    Some("protocol") => (RemotePhase::Failed, Some(RemoteFailureReason::Protocol)),
                    Some("network") => (
                        RemotePhase::Reconnecting,
                        Some(RemoteFailureReason::Network),
                    ),
                    Some("server") => (
                        RemotePhase::Reconnecting,
                        Some(RemoteFailureReason::RemoteServer),
                    ),
                    Some(_) => (RemotePhase::Failed, Some(RemoteFailureReason::Local)),
                    None => (RemotePhase::Stopped, None),
                }
            };
            RemoteStatus {
                phase,
                reason,
                recovery: reconnecting.then(|| RecoveryProgress {
                    attempt: START_RETRY_ATTEMPTS.load(Ordering::Acquire),
                    max_attempts: None,
                    since: START_RETRY_SINCE.load(Ordering::Acquire),
                    next_retry_at: match START_RETRY_NEXT_AT.load(Ordering::Acquire) {
                        0 => None,
                        value => Some(value),
                    },
                }),
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
    let Some((tx, pair_id, connected, drop_counters)) = ({
        let guard = STATE.lock().unwrap();
        guard.as_ref().map(|s| {
            (
                s.event_tx.clone(),
                s.pair_id.clone(),
                s.client.connection_state() == async_nats::connection::State::Connected,
                s.drop_counters.clone(),
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
        if let Some(line) = drop_counters.record_drop(
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
        if let Some(line) = drop_counters.record_drop(
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
    if let Some(line) = drop_counters.report_recovery() {
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
                if let Some(line) = EVENT_PUBLISH_EPISODE.record("event_publish", error) {
                    eprintln!("{line}");
                }
                break;
            } else if let Some(line) = EVENT_PUBLISH_EPISODE.recovered() {
                eprintln!("{line}");
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
                if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("heartbeat_publish", e) {
                    eprintln!("{line}");
                }
                return;
            } else if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.recovered() {
                eprintln!("{line}");
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
                        if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e) {
                            eprintln!("{line}");
                        }
                        return;
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
                    if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e) {
                        eprintln!("{line}");
                    }
                    return;
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
            // Credential scheduling owns only credential expiry. Critical task
            // and transport health are exclusively decided by
            // RemoteSupervisor, preventing two loops from replacing the same
            // generation concurrently.
            loop {
                tokio::time::sleep(refresh_tick()).await;
                let generation_active = STATE.lock().unwrap().as_ref().is_some_and(|state| {
                    state.pair_id == pair_id
                        && Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
                });
                if !generation_active {
                    return;
                }
                if STATE
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|state| state.nats_health.is_terminal())
                {
                    // A deterministic authorization/configuration failure does
                    // not improve by rotating the same credentials forever.
                    return;
                }
                // Refresh this far ahead of expiry (a production tick); kept
                // independent of the test-shrunk tick so the due path stays
                // reachable under test timing.
                let refresh_due =
                    pairing::refresh_delay(&creds) < std::time::Duration::from_secs(15);
                if refresh_due {
                    CREDENTIAL_REFRESHING.store(true, Ordering::Release);
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
                    let generation_active = STATE.lock().unwrap().as_ref().is_some_and(|state| {
                        state.pair_id == pair_id
                            && Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
                    });
                    if !generation_active {
                        return;
                    }
                    eprintln!("remote: pairing was revoked on the server [PA001]; stopping bridge");
                    *LAST_ERROR_CODE.lock().unwrap() = Some("revoked".to_string());
                    CREDENTIAL_REFRESHING.store(false, Ordering::Release);
                    let _ = pairing::clear_creds();
                    let _ = stop();
                    return;
                }
                Err(error) => {
                    if let Some(line) = CREDENTIAL_EPISODE.record("credential_network", error) {
                        eprintln!("{line}");
                    }
                    CREDENTIAL_REFRESHING.store(false, Ordering::Release);
                    tokio::time::sleep(refresh_tick()).await;
                    continue;
                }
            };
            let connected_nats = match connect_nats(&refreshed, true).await {
                Ok(connection) => connection,
                Err(crate::AppError::RemoteAuthorization(error)) => {
                    eprintln!("remote: refreshed NATS credential rejected [AU001]: {error}");
                    if let Some(state) = STATE.lock().unwrap().as_ref() {
                        state
                            .nats_health
                            .service_config_error
                            .store(true, Ordering::Release);
                    }
                    *LAST_ERROR_CODE.lock().unwrap() = Some("service_authorization".to_string());
                    CREDENTIAL_REFRESHING.store(false, Ordering::Release);
                    return;
                }
                Err(error) => {
                    if let Some(line) = CREDENTIAL_EPISODE.record("credential_connect", error) {
                        eprintln!("{line}");
                    }
                    CREDENTIAL_REFRESHING.store(false, Ordering::Release);
                    tokio::time::sleep(refresh_tick()).await;
                    continue;
                }
            };
            let client = connected_nats.client;
            let nats_health = connected_nats.health;
            let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
            let new_event = spawn_event_publisher(client.clone(), event_rx);
            let (command_ready_tx, command_ready_rx) = tokio::sync::oneshot::channel();
            let new_cmd = tokio::spawn(commands::command_loop_with_ready(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
                handshake_state.clone(),
                Some(command_ready_tx),
            ));
            let (transfer_ready_tx, transfer_ready_rx) = tokio::sync::oneshot::channel();
            let new_transfer = transfer::spawn_transfer_loop_with_ready(
                client.clone(),
                pair_id.clone(),
                handshake_state.active_flag(),
                Some(transfer_ready_tx),
            );
            let new_heartbeat = spawn_presence_heartbeat(
                client.clone(),
                pair_id.clone(),
                handshake_state.bridge_instance_id().to_string(),
            );
            tokio::task::yield_now().await;
            let readiness = async {
                command_ready_rx.await.map_err(|_| ())?;
                transfer_ready_rx.await.map_err(|_| ())?;
                client
                    .publish(
                        format!("p.{pair_id}.presence"),
                        serde_json::to_vec(&light_presence_payload(
                            &pair_id,
                            handshake_state.bridge_instance_id(),
                        ))
                        .unwrap_or_default()
                        .into(),
                    )
                    .await
                    .map_err(|_| ())?;
                client.flush().await.map_err(|_| ())
            };
            let readiness_failed = !matches!(
                tokio::time::timeout(std::time::Duration::from_secs(10), readiness).await,
                Ok(Ok(()))
            );
            if readiness_failed {
                new_event.abort();
                new_cmd.abort();
                new_transfer.abort();
                new_heartbeat.abort();
                CREDENTIAL_REFRESHING.store(false, Ordering::Release);
                tokio::time::sleep(refresh_tick()).await;
                continue;
            }
            // Hold the STATE lock across the generation check AND the creds
            // save: saving outside the lock raced `unpair()` (stop → clear
            // creds) and could resurrect a just-revoked credential file.
            let mut guard = STATE.lock().unwrap();
            let Some(state) = guard.as_mut().filter(|state| {
                state.pair_id == pair_id
                    && Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
            }) else {
                new_event.abort();
                new_cmd.abort();
                new_transfer.abort();
                new_heartbeat.abort();
                CREDENTIAL_REFRESHING.store(false, Ordering::Release);
                return;
            };
            if let Err(error) = pairing::save_creds(&refreshed) {
                eprintln!("remote: save refreshed credential failed [LC004]: {error}");
            }
            let old_cmd = std::mem::replace(&mut state.cmd_task, new_cmd);
            let old_transfer = std::mem::replace(&mut state.transfer_task, new_transfer);
            let old_heartbeat = std::mem::replace(&mut state.heartbeat_task, new_heartbeat);
            let old_event = std::mem::replace(&mut state.event_task, new_event);
            state.event_tx = event_tx;
            state.client = client;
            state.nats_health = nats_health;
            state.nats_url = refreshed.nats_url;
            old_cmd.abort();
            old_transfer.abort();
            old_heartbeat.abort();
            // The old event drain is deliberately NOT aborted: dropping the
            // handle detaches it, and it exits on its own after flushing its
            // backlog — no event gap at the swap point.
            drop(old_event);
            if let Some(line) = CREDENTIAL_EPISODE.recovered() {
                eprintln!("{line}");
            }
            CREDENTIAL_REFRESHING.store(false, Ordering::Release);
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

/// The web remote client is intentionally available only against the test
/// platform. Production and custom platform URLs retain mobile remote control
/// but never bind the local HTTP listener.
fn web_client_enabled_for_platform(platform_url: &str) -> bool {
    platform_url == crate::future_platform::TEST_PLATFORM_URL
}

fn web_client_enabled() -> bool {
    // Remote integration tests use local mock platform URLs while exercising
    // the listener; production code always checks the actual environment.
    #[cfg(test)]
    {
        true
    }
    #[cfg(not(test))]
    {
        web_client_enabled_for_platform(&crate::future_platform::current_platform_url())
    }
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

        // Episode progress is reported only at power-of-two attempts.
        for i in 1..=10 {
            let line = counters.record_drop("queue full", "tool_delta", "s1", 1_000 + i);
            assert_eq!(
                line.is_some(),
                i.is_power_of_two(),
                "unexpected reporting decision for drop {i}"
            );
        }
        assert_eq!(counters.reports.load(Ordering::Relaxed), 4);
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

    #[test]
    fn web_client_is_limited_to_the_test_platform() {
        assert!(web_client_enabled_for_platform(
            crate::future_platform::TEST_PLATFORM_URL
        ));
        assert!(!web_client_enabled_for_platform(
            crate::future_platform::PRODUCTION_PLATFORM_URL
        ));
        assert!(!web_client_enabled_for_platform(
            "https://custom.example.com"
        ));
    }

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
            generation_id: 1,
            client,
            nats_health: Arc::new(NatsHealth::default()),
            nats_url: nats.url().to_string(),
            pair_id: pair_id.to_string(),
            desktop_id: format!("desktop_{}", unique("rt")),
            desktop_public_key: "UPUBKEY".to_string(),
            bridge_instance_id: format!("bridge_{}", unique("rt")),
            event_tx,
            drop_counters: Arc::new(DropCounters::new()),
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

    #[test]
    fn nats_health_classifies_recoverable_and_terminal_events() {
        let _home = HomeGuard::new("remote-nats-health");
        let health = NatsHealth::default();
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::MaxReconnects,
        ));
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());

        health.handle_event(&async_nats::Event::Connected);
        assert!(!health.needs_reconnect());

        // async-nats wraps reconnect-time authorization errors in the client
        // event rather than forwarding the server event. This must trigger a
        // generation refresh instead of leaving an apparently live bridge in
        // an unbounded reconnect/log loop.
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("authorization violation".to_string()),
        ));
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("authorization violation".to_string()),
        ));
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());
        assert!(health
            .authorization_rejection_logged
            .load(Ordering::Acquire));

        health.handle_event(&async_nats::Event::Connected);
        assert!(!health.needs_reconnect());
        assert!(!health
            .authorization_rejection_logged
            .load(Ordering::Acquire));

        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("IO error".to_string()),
        ));
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("IO error".to_string()),
        ));
        assert!(!health.needs_reconnect());

        health.handle_event(&async_nats::Event::Connected);

        // The same rejection on the generation created by the reactive
        // refresh is terminal service authorization.
        health
            .credential_was_refreshed
            .store(true, Ordering::Release);
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::AuthorizationViolation,
        ));
        health.handle_event(&async_nats::Event::Connected);
        assert!(
            health.is_terminal(),
            "reconnect must not hide an auth failure"
        );
    }

    #[test]
    fn power_events_preserve_intent_and_only_resume_rebuilds_a_generation() {
        assert_eq!(
            power_transition(PowerEvent::Suspend, true),
            PowerTransition {
                stop_generation: true,
                start_generation: false,
                rotate_epoch: false,
            }
        );
        assert_eq!(
            power_transition(PowerEvent::Resume, true),
            PowerTransition {
                stop_generation: false,
                start_generation: true,
                rotate_epoch: true,
            }
        );
        for event in [PowerEvent::Suspend, PowerEvent::Resume] {
            assert_eq!(
                power_transition(event, false),
                PowerTransition {
                    stop_generation: false,
                    start_generation: false,
                    rotate_epoch: false,
                }
            );
        }
    }

    #[test]
    fn failure_logs_are_bounded_across_alternating_episodes() {
        // The per-episode counter alone is insufficient: alternating category
        // names used to restart it indefinitely. This unique test category
        // exercises the process-level, 24-hour quota.
        let category = "test_alternating_failure_quota";
        for _ in 0..MAX_FAILURE_LOGS_PER_CATEGORY {
            assert!(permit_failure_log(category));
        }
        assert!(!permit_failure_log(category));
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
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert_eq!(current.reason, None);
        assert_eq!(current.pair_id, "");

        pairing::save_creds(&test_creds("pair_stopped", "nats://127.0.0.1:1", 3600)).unwrap();
        *LAST_ERROR_CODE.lock().unwrap() = Some("revoked".to_string());
        let current = status();
        assert_eq!(current.reason, Some(RemoteFailureReason::CredentialRevoked));
        assert_eq!(current.pair_id, "pair_stopped");

        // A transient first connect failure starts the process-lifetime retry
        // worker before `STATE` exists. It is reconnecting, not a final error.
        *LAST_ERROR_CODE.lock().unwrap() = Some("network".to_string());
        START_REQUESTED.store(true, Ordering::Release);
        START_RETRY_RUNNING.store(true, Ordering::Release);
        let current = status();
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(current.reason, Some(RemoteFailureReason::Network));

        START_RETRY_RUNNING.store(false, Ordering::Release);
        START_REQUESTED.store(false, Ordering::Release);
        *LAST_ERROR_CODE.lock().unwrap() = None;
        pairing::clear_creds().unwrap();
    }

    #[test]
    fn stop_without_state_is_an_empty_noop() {
        let _home = HomeGuard::new("remote-stop-empty");
        let stopped = stop();
        assert!(!matches!(stopped.phase, RemotePhase::Ready));
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
        assert!(matches!(current.phase, RemotePhase::Ready));
        assert_eq!(current.pairing_code.as_deref(), Some("code-123"));
        assert_eq!(current.reason, None);

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
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(
            current.reason,
            Some(RemoteFailureReason::GenerationUnhealthy)
        );

        // Only after the bounded automatic budget is exhausted does the UI
        // receive an actionable reconnect-required error.
        RUNTIME_RECONNECT_ATTEMPTS.store(MAX_RUNTIME_RECONNECT_ATTEMPTS, Ordering::Release);
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(
            current.reason,
            Some(RemoteFailureReason::GenerationUnhealthy)
        );

        // Authorization/configuration failures are terminal for this
        // generation: do not spend the reconnect budget on the same invalid
        // service configuration.
        {
            let guard = STATE.lock().unwrap();
            let state = guard.as_ref().unwrap();
            state
                .nats_health
                .service_config_error
                .store(true, Ordering::Release);
        }
        RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert!(!matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(
            current.reason,
            Some(RemoteFailureReason::ServiceAuthorization)
        );
        STATE
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .nats_health
            .service_config_error
            .store(false, Ordering::Release);

        // The reconnect-path authorization shape must never leave the UI on
        // "connected" merely because the old generation still owns STATE.
        {
            let guard = STATE.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .nats_health
                .handle_event(&async_nats::Event::ClientError(
                    async_nats::ClientError::Other("authorization violation".to_string()),
                ));
        }
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        {
            let guard = STATE.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .nats_health
                .handle_event(&async_nats::Event::Connected);
        }

        // The optional Web listener retries silently, then becomes actionable
        // only after its own reconnect budget is exhausted.
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.web_task = None;
            state.cmd_task = tokio::spawn(std::future::pending());
        }
        assert_eq!(status().warning_code, None);
        WEB_RECONNECT_ATTEMPTS.store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        assert_eq!(status().warning_code.as_deref(), Some("web_bind"));

        let stopped = stop();
        assert!(!matches!(stopped.phase, RemotePhase::Ready));
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

        // Full/closed queue → the queue-full drop path. Reset this runtime's
        // cross-generation counters so this drop is the first in its episode.
        {
            let mut guard = STATE.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.drop_counters.dropping.store(false, Ordering::Relaxed);
            state.drop_counters.dropped.store(0, Ordering::Relaxed);
            state.drop_counters.reports.store(0, Ordering::Relaxed);
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
        let mut drain = spawn_event_publisher(client.clone(), rx);
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
        // the critical publisher exits so the generation supervisor rebuilds
        // it under the shared system-failure budget.
        tx.send(EventPublish {
            subject: "p.pair_drain.evt.sess.huge".to_string(),
            payload: vec![b'x'; 9 * 1024 * 1024],
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut drain)
            .await
            .expect("publisher exits after a critical write failure")
            .expect("publisher not panicked");
        assert!(tx
            .send(EventPublish {
                subject: "p.pair_drain.evt.sess.after".to_string(),
                payload: b"{}".to_vec(),
            })
            .await
            .is_err());
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
        assert!(matches!(started.phase, RemotePhase::Ready));
        assert!(started.pairing_code.is_some());
        assert_eq!(started.web_url.as_deref(), Some("http://localhost:8022"));
        assert_eq!(started.reason, None);

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
        assert!(matches!(started.phase, RemotePhase::Ready));
        assert_eq!(started.web_url, None);
        assert_eq!(started.web_lan_url, None);
        assert_eq!(started.warning_code.as_deref(), Some("web_bind"));
        assert_eq!(status().warning_code, None);
        WEB_RECONNECT_ATTEMPTS.store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        assert_eq!(status().warning_code.as_deref(), Some("web_bind"));
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
        assert!(matches!(started.phase, RemotePhase::Ready));
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
        assert!(!matches!(started.phase, RemotePhase::Ready));
        assert_eq!(started.reason, Some(RemoteFailureReason::Network));
        // The code sticks for later status() polls.
        assert_eq!(status().reason, Some(RemoteFailureReason::Network));

        // Pairing issued but NATS unreachable → same categorized failure.
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.respond_pair_code("nats://127.0.0.1:9");
        let started = start(RemoteStartInput {})
            .await
            .expect("nats failure maps to status");
        assert_eq!(started.reason, Some(RemoteFailureReason::Network));

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
            phase: RemotePhase::Reconnecting,
            reason: Some(RemoteFailureReason::Network),
            ..empty()
        };
        let server = RemoteStatus {
            phase: RemotePhase::Reconnecting,
            reason: Some(RemoteFailureReason::RemoteServer),
            ..empty()
        };
        let revoked = RemoteStatus {
            phase: RemotePhase::Revoked,
            reason: Some(RemoteFailureReason::CredentialRevoked),
            ..empty()
        };
        assert!(retryable_start_status(&network));
        assert!(retryable_start_status(&server));
        assert!(!retryable_start_status(&revoked));
        assert!(!retryable_start_status(&RemoteStatus {
            phase: RemotePhase::Ready,
            reason: Some(RemoteFailureReason::Network),
            ..empty()
        }));
    }

    #[test]
    fn initial_nats_connect_errors_distinguish_authorization_from_network() {
        for kind in [
            async_nats::ConnectErrorKind::Authentication,
            async_nats::ConnectErrorKind::AuthorizationViolation,
        ] {
            assert!(matches!(
                classify_nats_connect_error(kind, "rejected".to_string()),
                crate::AppError::RemoteAuthorization(_)
            ));
        }
        for kind in [
            async_nats::ConnectErrorKind::Dns,
            async_nats::ConnectErrorKind::TimedOut,
            async_nats::ConnectErrorKind::Io,
        ] {
            assert!(matches!(
                classify_nats_connect_error(kind, "offline".to_string()),
                crate::AppError::RemoteTransport(_)
            ));
        }
    }

    #[test]
    fn desktop_reconnect_backoff_matches_the_shared_policy() {
        assert_eq!(reconnect_delay(0, 0.5), Duration::from_secs(1));
        assert_eq!(reconnect_delay(1, 0.5), Duration::from_secs(2));
        assert_eq!(reconnect_delay(2, 0.5), Duration::from_secs(4));
        assert_eq!(reconnect_delay(4, 0.5), Duration::from_secs(16));
        assert_eq!(reconnect_delay(5, 0.5), Duration::from_secs(30));
        assert!(reconnect_delay(20, 1.0) <= Duration::from_secs(30));
    }

    #[test]
    fn bridge_shared_state_survives_generation_swaps_but_rotates_epoch() {
        let _home = HomeGuard::new("remote-shared-generation");
        *BRIDGE_SHARED.lock().unwrap() = None;
        let first = shared_runtime("pair_shared", true, false);
        let same_credential_epoch = shared_runtime("pair_shared", true, false);
        assert!(Arc::ptr_eq(
            &first.reply_slots,
            &same_credential_epoch.reply_slots
        ));
        assert!(Arc::ptr_eq(
            &first.pairing_confirmed,
            &same_credential_epoch.pairing_confirmed
        ));
        assert_eq!(
            first.bridge_instance_id,
            same_credential_epoch.bridge_instance_id
        );

        let rebuilt = shared_runtime("pair_shared", true, true);
        assert!(Arc::ptr_eq(&first.reply_slots, &rebuilt.reply_slots));
        assert!(Arc::ptr_eq(
            &first.pairing_confirmed,
            &rebuilt.pairing_confirmed
        ));
        assert_ne!(first.bridge_instance_id, rebuilt.bridge_instance_id);
        *BRIDGE_SHARED.lock().unwrap() = None;
    }

    #[test]
    fn runtime_failure_budget_uses_a_ten_minute_window() {
        let _home = HomeGuard::new("remote-runtime-budget");
        RUNTIME_FAILURE_WINDOW_STARTED.store(0, Ordering::Release);
        RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
        assert_eq!(record_runtime_failure(1_000), 1);
        assert_eq!(record_runtime_failure(2_000), 2);
        assert_eq!(record_runtime_failure(3_000), 3);
        assert_eq!(record_runtime_failure(4_000), 4);
        assert_eq!(
            record_runtime_failure(1_000 + RUNTIME_FAILURE_WINDOW_MS + 1),
            1
        );
        RUNTIME_FAILURE_WINDOW_STARTED.store(0, Ordering::Release);
        RUNTIME_RECONNECT_ATTEMPTS.store(0, Ordering::Release);
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

        // STATE has the same pairing id but belongs to a newer bridge
        // generation → the old refresh worker must not overwrite it.
        let state = fake_state(&nats, "pair_gen").await;
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
    async fn credential_loop_does_not_compete_with_runtime_supervisor() {
        let _home = HomeGuard::new("remote-refresh-unhealthy");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        // Far-future expiry: transport health belongs to RemoteSupervisor and
        // must not cause this credential-only loop to replace a generation.
        let creds = test_creds("pair_sick", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();

        let state = fake_state(&nats, "pair_sick").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_sick".into());
        let handle = spawn_credential_refresh(
            "pair_sick".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        // Killing the broker must not call the token endpoint from this task.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(platform.requests().is_empty());
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
    async fn heartbeat_publish_failure_exits_for_generation_supervisor() {
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
        // A critical publisher failure ends the generation instead of logging
        // once per heartbeat forever.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(handle.is_finished());
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
