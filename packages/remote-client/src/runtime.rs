//! One explicitly owned controller runtime for multiple remote endpoints.
//!
//! The instance owns sessions, subscriptions, bounded reconnects, request/page
//! assembly and chunk waiters. The host supplies identity, credential storage,
//! platform calls and task registration. No process-global state or application
//! implementation is used here. Stop admission before shutdown; shutdown waits
//! for owned task future destruction within a single total deadline.

use super::credentials::{self as creds, PeerCreds};
use super::host::ClientHost;
use super::session::{self, PeerSession};
use super::tasks::TaskHandle;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures::FutureExt;
use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use std::time::Duration;
use tokio::sync::Mutex;

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
    /// Whether the *user* stopped this host (its Connect/Disconnect control),
    /// as opposed to it being unreachable on its own.
    ///
    /// The distinction is the whole reason this is on the wire: a caller that
    /// keeps every paired host live (the app's own liveness pass) must retry a
    /// host that dropped and must *not* resurrect one the user deliberately
    /// turned off. Connecting it clears the mark — whichever call did so, its own
    /// Connect, a command addressed to it, or "reconnect all" — so the automatic
    /// pass can never undo a deliberate stop; the same rule the supervisor
    /// follows by being aborted on disconnect.
    pub stopped: bool,
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
    pub control: Option<session::ControlState>,
    pub holder: Option<session::Holder>,
}

/// Where pushed events go. Injected rather than hard-coded to Tauri so the
/// runtime stays testable and the desktop app is not the only possible host.
pub type Emitter = std::sync::Arc<dyn Fn(PeerEvent) + Send + Sync>;

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
    /// `control` announces a change of controller immediately; presence also
    /// restates control and its holder on each heartbeat.
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
    /// One session per connected host.
    ///
    /// Behind an `Arc` so a caller can take it out of the map and await a round
    /// trip *after* dropping the runtime mutex. Every one of these methods is
    /// either a quick read of the map (`list`) or a mutation of another host's
    /// entry, and a command's round trip is bounded by `COMMAND_TIMEOUT` — up to
    /// three attempts for a retryable read. Holding the mutex across that wait
    /// made one host's timeout stall every other host's list read: the device
    /// picker kept drawing its last answer while the Remote page's first read
    /// was still blocked, so a host that had merely dropped looked removed
    /// rather than offline.
    live: HashMap<String, std::sync::Arc<PeerSession>>,
    epochs: HashMap<String, u64>,
    /// The subscription task per host, aborted on disconnect so a host the user
    /// removed cannot keep emitting into a UI that has forgotten it.
    tasks: HashMap<String, TaskHandle>,
    /// The presence-beat task per host.
    ///
    /// Separate from `tasks` because it does not stand or fall with the stream:
    /// a host this client watches without an emitter still has to be told the
    /// client is here, or its Remote page would draw this device as gone while
    /// it is connected.
    beats: HashMap<String, TaskHandle>,
    /// The reconnect task per host, aborted on an explicit disconnect so a
    /// deliberate stop cannot be undone by a retry that was already scheduled.
    supervisors: HashMap<String, TaskHandle>,
    /// Where a host's pushes go, remembered for as long as the app is watching
    /// it.
    ///
    /// Held here rather than captured by one connection's stream task because a
    /// host outlives any single socket: an automatic reconnect has to re-attach
    /// the stream to *something*, and a supervisor that reconnected without an
    /// emitter produced an invisible connection — the host was `connected` and
    /// every command worked, but no push (a presence tick, a catalogue
    /// snapshot, a streamed reply) ever reached the UI. Only the command layer
    /// supplies one, and it stays until the pairing is removed.
    emitters: HashMap<String, Emitter>,
    /// The hosts the user has explicitly disconnected. See `PeerSummary::stopped`.
    stopped: std::collections::HashSet<String>,
    /// The last failure per host. Kept after a disconnect so the list can say
    /// *why* a host is not connected instead of only that it is not.
    errors: HashMap<String, String>,
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
    /// How many retries each host's supervisor has **dispatched** (test-only).
    ///
    /// This is the quantity `reconnect_attempt_budget` bounds: one increment per
    /// wake-up that actually tries the host again. Counting something else makes
    /// a budget assertion measure that other thing. Sockets are the trap here —
    /// a suite can observe accepted connections, but one dispatched attempt is
    /// not one socket: a library-internal reconnect, a second loop, or a stale
    /// loop dialling a port the OS has since reused all move a socket count
    /// without moving the retry policy at all.
    ///
    /// Per host rather than per loop, because two loops for one host is exactly
    /// the defect a budget check should catch: they share this counter, so the
    /// count would run past the budget instead of agreeing with it.
    #[cfg(any(test, feature = "test-support"))]
    retry_attempts: HashMap<String, std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

/// The retry schedule for a dropped connection, in seconds.
///
/// Mirrors the contract the phone implements: bounded backoff with jitter, and
/// a total window after which the client stops trying and reports a terminal
/// failure. An unbounded retry against a host that will never come back is
/// indistinguishable, to the user, from a hung app.
const RECONNECT_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

#[cfg(not(any(test, feature = "test-support")))]
/// The unit the schedule's numbers are counted in.
///
/// Seconds in production; milliseconds under test, so the suite does not spend
/// real minutes waiting out backoff. The *shape* being exercised is the real
/// one either way — scaling the unit is what keeps the schedule itself honest.
fn delay_unit() -> std::time::Duration {
    std::time::Duration::from_secs(1)
}
#[cfg(any(test, feature = "test-support"))]
fn delay_unit() -> std::time::Duration {
    std::time::Duration::from_millis(1)
}

/// How often a connected client proves it is here.
///
/// Production sends at [`session::CLIENT_PRESENCE_BEAT`]; the test build ticks
/// far faster so a suite can observe a beat without waiting out a production
/// quarter-minute. The *shape* is the same either way — the client beats on a
/// timer that stops with the connection — and the host's lease is asserted
/// against the production number where it lives
/// (`future-remote-server::supervisor::link`).
#[cfg(not(any(test, feature = "test-support")))]
fn presence_beat_interval() -> std::time::Duration {
    session::CLIENT_PRESENCE_BEAT
}
#[cfg(any(test, feature = "test-support"))]
fn presence_beat_interval() -> std::time::Duration {
    std::time::Duration::from_millis(100)
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

/// How often a live generation re-reads its grant.
///
/// A wake-up cadence, never the criterion: whether to rotate is decided by
/// [`PeerCreds::needs_refresh`] on the expiry the store holds, and the wait
/// before each check is the grant's own deadline (see [`rotation_wait`]). The
/// tick only bounds how long a grant the store changed by another path can go
/// unnoticed.
#[cfg(not(any(test, feature = "test-support")))]
fn rotation_tick() -> std::time::Duration {
    std::time::Duration::from_secs(15)
}
#[cfg(any(test, feature = "test-support"))]
fn rotation_tick() -> std::time::Duration {
    std::time::Duration::from_millis(25)
}

/// How long the rotation worker sleeps before looking at the grant again.
///
/// The wait is the credential's own deadline — its declared expiry minus
/// [`creds::REFRESH_LEAD_SECS`] — capped by the tick, so a grant the store
/// changed from elsewhere (a re-pair, another writer) is re-read within one
/// cadence. A grant that is already due has a zero wait: it is rotated on the
/// next pass rather than after a fixed delay. A credential with no declared
/// expiry has nothing to schedule and is re-read at the tick.
fn rotation_wait(expires_at: i64, now: i64) -> std::time::Duration {
    let tick = rotation_tick();
    if expires_at <= 0 {
        return tick;
    }
    let due_in = expires_at - creds::REFRESH_LEAD_SECS - now;
    std::time::Duration::from_secs(due_in.max(0) as u64).min(tick)
}

/// The per-connection traffic keys of one generation.
///
/// This is the generation's identity: a newer connection always has its own
/// channel, so a pointer comparison answers "is this task still serving the host
/// I was built for?" without inventing a second, independent generation number
/// that could drift from the socket it describes.
type SessionChannel = std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>;

/// How many attempts form a reconnect burst before a quiet cooldown.
///
/// The schedule's length times three: the window is what makes a failure
/// enter cooldown. Transient offline hosts stay registered and recover without
/// user action; explicit disconnect and authorization refusal remain terminal.
///
/// Public because it is a contract, not an implementation detail: a test that
/// asserts a loop stopped after "the full window" has to be able to name the
/// window instead of hard-coding a number that would silently stop matching the
/// schedule it claims to exercise.
pub fn reconnect_attempt_budget() -> usize {
    RECONNECT_DELAYS_SECS.len() * 3
}

/// Whether a failed connect should schedule a bounded retry.
///
/// Three conditions, all necessary, and each has a distinct reason:
///
/// * **Transient** (`AppError::RemoteTransport`) — the network is the problem and
///   a retry can help. An `AppError::RemoteAuthorization` is a rejected
///   credential: retrying it forever repairs nothing (that variant's own doc
///   says so), and it would also hide a re-pair the user has to do.
/// * **Not user-stopped** — a host the user disconnected must stay disconnected.
///   The stop is only cleared by a successful connect, so if a retry were allowed
///   to run here it would resurrect the link the user just closed.
/// * **Not already retrying** — the loop's own `resume` comes back through
///   `connect_with_emitter`, so without this a failing retry would spawn a second
///   loop per attempt.
pub fn should_retry_connect(
    error: &crate::Error,
    user_stopped: bool,
    already_retrying: bool,
) -> bool {
    matches!(error, crate::Error::RemoteTransport(_)) && !user_stopped && !already_retrying
}

/// The stream half of a connection: the traffic-key handle, the subscriptions
/// the event task drains and the sink they go to. Bundled rather than returned
/// as a tuple now that a fourth lane (the host-wide `state.>`) is among them.
struct StreamParts {
    channel: std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    subscriptions: super::session::Subscriptions,
    emitter: Emitter,
    control: std::sync::Arc<std::sync::Mutex<session::Control>>,
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
            control: peer_session.control_for_stream(),
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
const CONTROL_LANE: &str = "control";
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
pub fn event_session_id<'a>(subject: &'a str, pair_id: &str) -> Option<&'a str> {
    let rest = subject.strip_prefix("p.")?;
    let rest = rest.strip_prefix(pair_id)?;
    let session = rest.strip_prefix(".evt.")?;
    (!session.is_empty()).then_some(session)
}

/// Which kind of host-wide lane a `state.>` subject names.
///
/// The host publishes four things under `state.>`: the event mirror, the
/// sessions snapshot, the activity packet and the workspaces snapshot. Only
/// `state.events` is read as an event (and only while the event lane is
/// narrowed); the snapshots and the packet are forwarded as catalog updates.
enum StateEventKind {
    Events,
    Sessions,
    /// The rows whose activity timestamp moved, for a client that declared
    /// `catalog_activity_v1`. A packet, not a snapshot: it names only the rows
    /// that moved, so a reader must not treat the rest as absent.
    Activity,
    Workspaces,
    Control,
    Other,
}

fn state_event_kind(subject: &str, pair_id: &str) -> StateEventKind {
    if subject == format!("p.{pair_id}.state.events") {
        StateEventKind::Events
    } else if subject == format!("p.{pair_id}.state.sessions") {
        StateEventKind::Sessions
    } else if subject == format!("p.{pair_id}.state.activity") {
        StateEventKind::Activity
    } else if subject == format!("p.{pair_id}.state.workspaces") {
        StateEventKind::Workspaces
    } else if subject == format!("p.{pair_id}.state.control") {
        StateEventKind::Control
    } else {
        StateEventKind::Other
    }
}

/// `transfer_id` and chunk index from a `xfer.down` subject.
///
/// Returns `None` for anything else, including a subject from another pair:
/// the host's own layout is `p.{pair}.xfer.down.{transfer}.chunk.{index}`, and a
/// chunk whose shape does not match that is not something to guess at.
pub fn parse_chunk_subject(subject: &str) -> Option<(String, u64)> {
    let parts: Vec<&str> = subject.split('.').collect();
    let ["p", _pair, "xfer", "down", transfer_id, "chunk", index] = parts.as_slice() else {
        return None;
    };
    index
        .parse::<u64>()
        .ok()
        .map(|index| ((*transfer_id).to_string(), index))
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
) -> Result<(), crate::Error> {
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

fn chunk_string(chunk: &Value, key: &str) -> Result<String, crate::Error> {
    chunk
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(page_shape_error)
}

fn chunk_u64(chunk: &Value, key: &str) -> Result<u64, crate::Error> {
    chunk
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(page_shape_error)
}

fn page_shape_error() -> crate::Error {
    crate::Error::Message("remote_read_invalid_chunk".into())
}

/// Note a transport failure and make sure the link is being retried.
///
/// Shared by the session-keyed command lane and the transfer lane: both talk to
/// the same socket, so both have to leave the same state behind when it dies. A
/// second copy is a second place for the recovery to drift.
/// Whether an error is a *command or handshake request* timing out, rather than
/// the transport underneath it dying.
///
/// Matched on the phrase both request-level paths produce (`session::exchange`'s
/// own budget: `"Remote request timed out"`, and `async-nats`'s inner one:
/// `"Remote request failed: request timed out: deadline has elapsed"`).
/// Deliberately **not** "timed out" alone: `"Failed to connect to NATS: timed
/// out"` is the socket-connect leg, where there is no session and nothing to
/// probe.
///
/// The `handshake` exclusion is load-bearing and was added after measuring it:
/// the Noise leg's *inner* timeout reads `"Remote handshake failed: request timed
/// out: deadline has elapsed"`, which **contains** the request phrase — so a
/// plain substring match answers "yes" for a handshake. It is inert today (the
/// only caller, `on_transport_failure`, never sees a handshake error: those come
/// out of `connect_with_emitter`, not from a live connection's request path), and
/// that is exactly why it deserves closing here rather than waiting for someone
/// to widen a caller.
pub fn is_request_timeout(error: &crate::Error) -> bool {
    match error {
        crate::Error::RemoteTransport(message) => {
            message.contains("request timed out") && !message.contains("handshake")
        }
        _ => false,
    }
}

/// The timeout for one chunk pull, matching the phone's own.
///
/// One value for the app and for the suite. It used to be two (500 ms under
/// `cfg(test)`), to keep the suite from spending 15 s on the arm that catches a
/// host which acknowledged a publish that never arrived — but that made the
/// suite's *passing* tests depend on the host scheduling a delivery inside
/// 500 ms, which is a budget no test can assert and a loaded machine can miss
/// (FLAKES.md item 8: CI reported `peer_not_connected` after a recovered 2.4 s
/// failure, and this host's full-suite runs have single tests at 20 s). The one
/// test that needs the give-up arm passes its budget to
/// [`pull_chunk_within`].
fn chunk_pull_timeout() -> Duration {
    const TIMEOUT: Duration = Duration::from_secs(15);
    TIMEOUT
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}
type ChunkKey = (String, String, u64);
struct ChunkWaiter {
    registration: Arc<()>,
    sender: tokio::sync::oneshot::Sender<Vec<u8>>,
}
pub(crate) struct ChunkRegistration {
    client: Client,
    key: ChunkKey,
    registration: Arc<()>,
}
impl Drop for ChunkRegistration {
    fn drop(&mut self) {
        let mut chunks = self
            .client
            .inner
            .chunks
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if chunks
            .get(&self.key)
            .is_some_and(|waiter| Arc::ptr_eq(&waiter.registration, &self.registration))
        {
            chunks.remove(&self.key);
        }
    }
}

struct ClientInner {
    state: Mutex<Runtime>,
    host: Arc<dyn ClientHost>,
    /// Synchronous registration lets a dropped caller future remove its waiter
    /// without spawning background cleanup or removing a newer retry.
    chunks: std::sync::Mutex<HashMap<ChunkKey, ChunkWaiter>>,
    tasks: std::sync::Mutex<Vec<TaskHandle>>,
    closed: std::sync::atomic::AtomicBool,
    /// Whether the grant-rotation worker has been started. One per client: it
    /// rotates every live host, and a second one would refresh the same grant
    /// twice.
    rotation_started: std::sync::atomic::AtomicBool,
    /// The grant expiry each host was last *rotated for*, so a refresh that
    /// hands back a grant inside the refresh window cannot make the worker
    /// reconnect in a tight loop: the same grant is rotated once, and again only
    /// when the store moves to a different one.
    rotated_for: std::sync::Mutex<HashMap<String, i64>>,
    stopping: tokio::sync::watch::Sender<bool>,
    #[cfg(any(test, feature = "test-support"))]
    inject_connection_lost: std::sync::atomic::AtomicBool,
}
impl Client {
    #[cfg(any(test, feature = "test-support"))]
    pub fn inject_connection_loss_for_test(&self) {
        self.inner
            .inject_connection_lost
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub async fn take_session_for_test(&self, desktop: &str) -> Option<Arc<PeerSession>> {
        self.inner.state.lock().await.live.remove(desktop)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub async fn stamp_live_for_test(
        &self,
        desktop: &str,
        data: Value,
    ) -> Result<Value, crate::Error> {
        self.stamp_live(desktop, data).await
    }
    #[cfg(any(test, feature = "test-support"))]
    pub async fn stream_attaches_for_test(
        &self,
        session: &PeerSession,
        emitter: Emitter,
        desktop: &str,
        visible: Option<&str>,
    ) -> bool {
        attach_stream(session, emitter, desktop, visible)
            .await
            .is_some()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub async fn spawn_fixture_stream(
        &self,
        session: Arc<PeerSession>,
        subscriptions: session::Subscriptions,
        desktop: &str,
        emitter: Emitter,
        capacity: usize,
    ) -> (TaskHandle, tokio::sync::watch::Sender<Option<String>>) {
        // The production stream only accepts pushes from the currently owned
        // session. Give the fixture the same ownership as a real connection.
        self.inner
            .state
            .lock()
            .await
            .live
            .insert(desktop.to_string(), session.clone());
        let (sender, receiver) = tokio::sync::watch::channel(None);
        let task = self.spawn_event_stream_capped(
            desktop.to_string(),
            session.pair_id().to_string(),
            session.event_lane(),
            None,
            receiver,
            StreamParts {
                channel: session.channel_for_stream(),
                subscriptions,
                emitter,
                control: session.control_for_stream(),
            },
            capacity,
        );
        (task, sender)
    }
    pub fn new(host: Arc<dyn ClientHost>) -> Self {
        Self {
            inner: Arc::new(ClientInner {
                state: Mutex::new(Runtime::default()),
                host,
                chunks: std::sync::Mutex::new(HashMap::new()),
                tasks: std::sync::Mutex::new(Vec::new()),
                closed: std::sync::atomic::AtomicBool::new(false),
                rotation_started: std::sync::atomic::AtomicBool::new(false),
                rotated_for: std::sync::Mutex::new(HashMap::new()),
                stopping: tokio::sync::watch::channel(false).0,
                #[cfg(any(test, feature = "test-support"))]
                inject_connection_lost: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }
    async fn begin_attempt(&self, desktop: &str) -> Result<u64, crate::Error> {
        self.begin_connect_attempt(desktop, false).await
    }
    async fn begin_connect_attempt(
        &self,
        desktop: &str,
        replace_supervisor: bool,
    ) -> Result<u64, crate::Error> {
        let mut state = self.inner.state.lock().await;
        self.ensure_running()?;
        if replace_supervisor {
            if let Some(supervisor) = state.supervisors.remove(desktop) {
                supervisor.abort();
            }
        }
        let epoch = state.epochs.entry(desktop.to_string()).or_default();
        *epoch = epoch.wrapping_add(1);
        Ok(*epoch)
    }
    fn check_attempt(
        &self,
        state: &Runtime,
        desktop: &str,
        epoch: u64,
    ) -> Result<(), crate::Error> {
        self.ensure_running()?;
        if state.epochs.get(desktop) == Some(&epoch) {
            Ok(())
        } else {
            Err(crate::Error::Message("peer_not_connected".into()))
        }
    }
    pub fn stop_admission(&self) {
        self.inner
            .closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.inner.stopping.send_replace(true);
        for task in self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
        {
            task.abort();
        }
    }
    pub fn is_stopped(&self) -> bool {
        self.inner.closed.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) async fn while_running<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, crate::Error>>,
    ) -> Result<T, crate::Error> {
        let mut stop = self.inner.stopping.subscribe();
        self.ensure_running()?;
        tokio::select! {
            result = future => result,
            _ = stop.changed() => Err(crate::Error::Message("remote_client_stopped".into())),
        }
    }
    pub(crate) async fn transfer_session(
        &self,
        desktop: &str,
    ) -> Result<(u64, Arc<PeerSession>), crate::Error> {
        let state = self.inner.state.lock().await;
        self.ensure_running()?;
        let session = state
            .live
            .get(desktop)
            .cloned()
            .ok_or_else(|| crate::Error::Message("peer_not_connected".into()))?;
        if state.stopped.contains(desktop) {
            return Err(crate::Error::Message("peer_not_connected".into()));
        }
        let epoch = state.epochs.get(desktop).copied().unwrap_or_default();
        Ok((epoch, session))
    }
    pub(crate) async fn check_transfer(
        &self,
        desktop: &str,
        epoch: u64,
        session: &Arc<PeerSession>,
    ) -> Result<(), crate::Error> {
        let state = self.inner.state.lock().await;
        self.check_attempt(&state, desktop, epoch)?;
        if state.stopped.contains(desktop)
            || !state
                .live
                .get(desktop)
                .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            return Err(crate::Error::Message("peer_not_connected".into()));
        }
        Ok(())
    }
    pub(crate) fn transfer_command_id(&self) -> String {
        self.inner.host.command_id()
    }
    pub(crate) async fn record_transfer_failure(
        &self,
        desktop: &str,
        session: &Arc<PeerSession>,
        error: &crate::Error,
    ) {
        let mut state = self.inner.state.lock().await;
        if state
            .live
            .get(desktop)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            self.on_transport_failure(&mut state, desktop, error);
        }
    }
    fn ensure_running(&self) -> Result<(), crate::Error> {
        if self.inner.closed.load(std::sync::atomic::Ordering::Acquire) {
            Err(crate::Error::Message("remote_client_stopped".into()))
        } else {
            Ok(())
        }
    }
    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) -> TaskHandle {
        let (task, wrapped) = crate::tasks::tracked(future.boxed());
        let mut tasks = self.inner.tasks.lock().unwrap_or_else(|p| p.into_inner());
        tasks.retain(|task| !task.is_finished());
        if self.inner.closed.load(std::sync::atomic::Ordering::Acquire) {
            task.abort();
            drop(wrapped);
        } else {
            tasks.push(task.clone());
            self.inner.host.spawn(wrapped);
        }
        task
    }
    /// Stop new work, cancel all owned tasks, and wait for their future drops
    /// within one total deadline. The instance cannot be restarted after stop.
    pub async fn shutdown(&self, budget: Duration) -> Result<(), crate::Error> {
        self.stop_admission();
        let deadline = tokio::time::Instant::now() + budget;
        let tasks =
            std::mem::take(&mut *self.inner.tasks.lock().unwrap_or_else(|p| p.into_inner()));
        for task in &tasks {
            task.abort();
        }
        let mut timed_out = false;
        for task in tasks {
            if tokio::time::timeout_at(deadline, task.completed())
                .await
                .is_err()
            {
                timed_out = true;
            }
        }
        let sessions = {
            let mut state = self.inner.state.lock().await;
            let sessions = state
                .live
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>();
            state.tasks.clear();
            state.beats.clear();
            state.supervisors.clear();
            state.emitters.clear();
            state.visible.clear();
            self.inner
                .chunks
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
            state.errors.clear();
            sessions
        };
        for session in sessions {
            // Closing the app is a departure, and the host is still listening: a
            // goodbye here is what keeps every host this machine was driving from
            // drawing it as present until the lease runs out. Bounded twice —
            // once by the goodbye's own budget, once by the caller's deadline.
            if tokio::time::timeout_at(deadline, session.leave("disconnected"))
                .await
                .is_err()
            {
                timed_out = true;
            }
            if tokio::time::timeout_at(deadline, session.close_socket())
                .await
                .is_err()
            {
                timed_out = true;
            }
        }
        if timed_out {
            Err(crate::Error::Message(
                "remote_client_cleanup_timed_out".into(),
            ))
        } else {
            Ok(())
        }
    }
    /// The live session for a host, cloned so the caller can await on it with the
    /// runtime mutex released.
    ///
    /// Both command lanes go through here, for two reasons that have to hold
    /// together:
    ///
    /// * **The lock is not held across the round trip.** A command's budget is
    ///   `session::COMMAND_TIMEOUT` — three attempts for a retryable read, so up to
    ///   half a minute against a host that has silently dropped. Every state read on
    ///   this module (`list()`, `connect_all`, `disconnect`, `set_visible_session`)
    ///   needs the same mutex, so holding it through that wait made one unreachable
    ///   host stall the *peer list* of every other host: the picker kept drawing its
    ///   last answer while the Remote page's first read was still blocked, and a
    ///   machine that had merely gone offline looked removed.
    /// * **The race the test hook forces is the one being closed.** The entry can
    ///   disappear between `ensure_connected` and this lookup (a concurrent command's
    ///   transport failure, or the user pressing Disconnect), so the answer is an
    ///   `Option` and each caller says `peer_not_connected` itself.
    async fn session_handle(
        &self,
        desktop_id: &str,
    ) -> Option<std::sync::Arc<session::PeerSession>> {
        let live = self.inner.state.lock().await;
        #[cfg(any(test, feature = "test-support"))]
        let live = self.forced_connection_loss(live, desktop_id);
        live.live.get(desktop_id).cloned()
    }
    #[cfg(any(test, feature = "test-support"))]
    /// Apply the one-shot [`INJECT_CONNECTION_LOST`] hook, if it is armed.
    ///
    /// Separate from [`session_handle`] so production never holds a `mut` guard it
    /// does not use: the entry can only be dropped here, in a test build.
    fn forced_connection_loss<'a>(
        &self,
        mut live: tokio::sync::MutexGuard<'a, Runtime>,
        desktop_id: &str,
    ) -> tokio::sync::MutexGuard<'a, Runtime> {
        if self
            .inner
            .inject_connection_lost
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            live.live.remove(desktop_id);
        }
        live
    }

    #[cfg(any(test, feature = "test-support"))]
    /// How many hosts are currently connected. Tests assert on counts rather than
    /// poking at the map, so the test does not have to hold the runtime lock while
    /// it reasons about the result.
    pub async fn live_count(&self) -> usize {
        self.inner.state.lock().await.live.len()
    }
    #[cfg(any(test, feature = "test-support"))]
    /// How many event-stream tasks are *running*.
    ///
    /// Finished handles stay in the map until something replaces them, so counting
    /// the map would report a task that has already returned.
    pub async fn stream_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .await
            .tasks
            .values()
            .filter(|handle| !handle.is_finished())
            .count()
    }
    #[cfg(any(test, feature = "test-support"))]
    /// How many retry loops are running.
    pub async fn supervisor_count(&self) -> usize {
        self.inner.state.lock().await.supervisors.len()
    }
    #[cfg(any(test, feature = "test-support"))]
    /// How many retries this host's supervisor has dispatched so far.
    ///
    /// The budget is counted in these, so a caller can assert a loop spent
    /// exactly its window instead of counting sockets (see `retry_attempts`).
    pub async fn retry_attempts(&self, desktop_id: &str) -> usize {
        self.inner
            .state
            .lock()
            .await
            .retry_attempts
            .get(desktop_id)
            .map_or(0, |count| count.load(std::sync::atomic::Ordering::SeqCst))
    }
    #[cfg(any(test, feature = "test-support"))]
    /// Record one dispatched retry for `desktop_id`.
    async fn note_retry_attempt(&self, desktop_id: &str) {
        let counter = {
            let mut state = self.inner.state.lock().await;
            state
                .retry_attempts
                .entry(desktop_id.to_string())
                .or_default()
                .clone()
        };
        // Incremented outside the lock: the loop takes it on every attempt and
        // the map itself only ever gains a key once per host.
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(any(test, feature = "test-support"))]
    /// Abort every retry loop and forget them.
    ///
    /// `teardown()` calls this: a supervisor started by a *failed connect* (or by a
    /// dropped connection) is not tied to any test's lifetime, and the test build's
    /// retry schedule runs on `delay_unit()` = **1 ms** — so a loop left behind is a
    /// CPU burner that hammers the in-process broker for every test that runs after
    /// it. Measured: without this, one starved run's red count rose from 9 to 18 when
    /// connect failures began scheduling retries (`docs/testing/flaky-tauri.md`
    /// §13.8.4), i.e. the fix's own leak was louder than the defect it fixed.
    pub async fn abort_supervisors_for_test(&self) {
        let mut live = self.inner.state.lock().await;
        for (_, supervisor) in live.supervisors.drain() {
            supervisor.abort();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    /// Stop a host's stream task while leaving the connection in place.
    ///
    /// The state a wedged or dead reader leaves behind: commands still work (the
    /// host answers on the reply subject through this client's own request path),
    /// but nothing is delivered from the subscriptions. That is exactly what a chunk
    /// pull's timeout exists for, and it cannot be produced from outside.
    pub async fn stop_stream_for_test(&self, desktop_id: &str) {
        let task = self.inner.state.lock().await.tasks.remove(desktop_id);
        if let Some(task) = task {
            task.abort();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    /// Close a live connection's socket without touching the retry state.///
    /// The state a process shutting down leaves behind: the subscription streams
    /// end, so a test can drive the stream task's own exit instead of racing a
    /// broker shutdown (a reconnecting client keeps its subscriptions open).
    pub async fn close_socket_for_test(&self, desktop_id: &str) {
        let session = self
            .inner
            .state
            .lock()
            .await
            .live
            .remove(desktop_id)
            .expect("a live session for the test to close");
        let _ = session.close_socket().await;
    }
    #[cfg(any(test, feature = "test-support"))]
    ///
    /// Pretend a retry loop is already running for this host.
    ///
    /// The state a transport failure leaves behind, so a test can drive "a connect
    /// found a retry loop and replaced it" without first having to induce the
    /// failure and wait for the loop to be installed.
    pub async fn install_supervisor_for_test(&self, desktop_id: &str) -> TaskHandle {
        // Register before the task can attempt a connection. A transient
        // failure otherwise sees no owner and starts a second retry loop.
        let mut state = self.inner.state.lock().await;
        // Retire a loop this call supersedes, exactly as an explicit connect does
        // (`begin_connect_attempt`). Overwriting the map entry alone would leave
        // the old loop running with no owner left to abort it: a second loop
        // dialling the same host, invisible to `supervisor_count` and immortal
        // for `abort_supervisors_for_test`. The handle's drop does not cancel the
        // task, so "replace the entry" is not "stop the old loop".
        if let Some(previous) = state.supervisors.remove(desktop_id) {
            previous.abort();
        }
        let handle = self.spawn_supervisor(desktop_id.to_string(), None);
        state
            .supervisors
            .insert(desktop_id.to_string(), handle.clone());
        handle
    }
    /// Renew the grant and persist it.
    ///
    /// Persisting is part of the operation, not a nicety: a rotated token that is
    /// only held in memory makes the *next* launch renew again from the token this
    /// process already replaced.
    async fn renew_credentials(
        &self,
        creds: &PeerCreds,
        epoch: u64,
    ) -> Result<PeerCreds, crate::Error> {
        let fresh = self.while_running(self.inner.host.refresh(creds)).await?;
        {
            let state = self.inner.state.lock().await;
            self.check_attempt(&state, &creds.desktop_id, epoch)?;
            if !self.inner.host.update_credentials(creds, &fresh)? {
                return Err(crate::Error::Message("peer_not_connected".into()));
            }
        }
        let mut renewed = creds.clone();
        renewed.user_jwt = fresh.user_jwt;
        renewed.nats_url = fresh.nats_url;
        renewed.nats_ws_url = fresh.nats_ws_url;
        renewed.jwt_expires_at = fresh.jwt_expires_at;
        Ok(renewed)
    }
    #[cfg(any(test, feature = "test-support"))]
    /// Forget a connection without the command path noticing — the state a socket
    /// drop leaves behind, so a test can drive the recovery path deterministically
    /// instead of racing a broker shutdown.
    pub async fn forget_connection_for_test(&self, desktop_id: &str) {
        self.inner.state.lock().await.live.remove(desktop_id);
    }
    /// Pair with a remote host from a pasted `futureos://remote/pair` link, then
    /// connect once so the caller learns immediately whether it worked.
    ///
    /// The link failures carry the mobile client's `PA*` support codes, so both
    /// platforms name the same fault the same way and a support conversation means
    /// the same thing on either.
    pub async fn pair_with_emitter(
        &self,
        invitation: &str,
        emitter: Option<Emitter>,
    ) -> Result<PeerSummary, crate::Error> {
        self.ensure_running()?;
        let parsed =
            future_remote_protocol::invitation::parse_invitation(invitation, self.now_secs())
                .map_err(|error| crate::Error::Remote {
                    status: 0,
                    code: Some(error.support_code().to_string()),
                    message: error.message().to_string(),
                })?;
        let epoch = self.begin_attempt(&parsed.desktop_id).await?;
        let device = nkeys::KeyPair::new_user();
        let (private, public) = future_remote_crypto::generate_identity()
            .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
        let device_id = self.inner.host.device_id()?;
        let claimed = self
            .while_running(self.inner.host.claim(
                &parsed,
                &device_id,
                &device.public_key(),
                &self.inner.host.connection_options().device_name,
            ))
            .await?;
        let mut creds = PeerCreds {
            pair_id: claimed.pair_id,
            desktop_id: parsed.desktop_id.clone(),
            device_id,
            nkey_seed: device
                .seed()
                .map_err(|error| crate::Error::Message(format!("generate device NKey: {error}")))?,
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
            let fresh = self.while_running(self.inner.host.refresh(&creds)).await?;
            creds.user_jwt = fresh.user_jwt;
            creds.nats_url = fresh.nats_url;
            creds.nats_ws_url = fresh.nats_ws_url;
            creds.jwt_expires_at = fresh.jwt_expires_at;
        }
        let desktop_id = creds.desktop_id.clone();
        {
            let state = self.inner.state.lock().await;
            self.check_attempt(&state, &desktop_id, epoch)?;
            self.inner.host.upsert(creds)?;
        }
        self.connect_with_emitter(&desktop_id, emitter).await
    }
    /// The book's view of one host, without any live state.
    fn paired_summary(&self, desktop_id: &str) -> Result<PeerSummary, crate::Error> {
        let book = self.inner.host.load()?;
        let peer = book
            .find(desktop_id)
            .ok_or_else(|| crate::Error::Message("peer_not_paired".into()))?;
        Ok(PeerSummary {
            desktop_id: peer.creds.desktop_id.clone(),
            pair_id: peer.creds.pair_id.clone(),
            name: peer.label.name.clone(),
            icon: peer.label.icon.clone(),
            connected: false,
            // Never stopped: this constructor describes a book entry, and the
            // callers that want the live answer read `list()`.
            stopped: false,
            error: None,
            bridge_instance_id: None,
            features: Vec::new(),
            // Not connected yet, so nothing has been heard from the host. `false`
            // rather than `true` keeps "unknown" on the safe side of the UI's
            // "everything is fine" check.
            agent_available: false,
            control: None,
            holder: None,
        })
    }
    /// Every paired host, whether or not it is currently connected.
    pub async fn list(&self) -> Result<Vec<PeerSummary>, crate::Error> {
        let book = self.inner.host.load()?;
        let live = self.inner.state.lock().await;
        Ok(book
            .peers
            .iter()
            .map(|peer| {
                let session = live.live.get(&peer.creds.desktop_id);
                let control = session.map(|session| session.control()).unwrap_or_default();
                PeerSummary {
                    desktop_id: peer.creds.desktop_id.clone(),
                    pair_id: peer.creds.pair_id.clone(),
                    name: peer.label.name.clone(),
                    icon: peer.label.icon.clone(),
                    connected: session.is_some(),
                    stopped: live.stopped.contains(&peer.creds.desktop_id),
                    error: live.errors.get(&peer.creds.desktop_id).cloned(),
                    bridge_instance_id: session
                        .map(|session| session.bridge_instance_id().to_string()),
                    features: session
                        .map(|session| session.features().to_vec())
                        .unwrap_or_default(),
                    agent_available: session.is_some_and(|session| session.agent_available()),
                    control: control.state,
                    holder: control.holder,
                }
            })
            .collect())
    }
    /// Open a connection unless one is already live.
    pub async fn ensure_connected(&self, desktop_id: &str) -> Result<PeerSummary, crate::Error> {
        self.ensure_running()?;
        if let Some(peer) = self
            .list()
            .await?
            .into_iter()
            .find(|peer| peer.desktop_id == desktop_id)
            .filter(|peer| peer.connected)
        {
            return Ok(peer);
        }
        self.connect(desktop_id).await
    }
    /// Connect (replacing any existing connection for this host). A failed attempt
    /// is recorded against the host so the list can explain it, then returned.
    pub async fn connect(&self, desktop_id: &str) -> Result<PeerSummary, crate::Error> {
        self.connect_with_emitter(desktop_id, None).await
    }
    /// Make every paired host live, and answer with the list as it stands.
    ///
    /// This is the client role's "everything I am paired with should be up": the
    /// user's own "reconnect all", and the only caller. It is deliberately
    /// unconditional about *stopped* hosts, because this call is what clears the
    /// stop a host was carrying — the user asking for every machine to come back
    /// includes the one they turned off. The app's automatic liveness pass must not
    /// use it for exactly that reason: it asks per host and skips the stopped ones
    /// (see `PeerSummary::stopped`).
    ///
    /// Already-live hosts are left alone, so calling it repeatedly is free, and one
    /// unreachable host cannot fail the others: its error is recorded against it
    /// and shown on its own row, which is the same treatment its own Connect button
    /// gets.
    pub async fn connect_all(
        &self,
        emitter: Option<Emitter>,
    ) -> Result<Vec<PeerSummary>, crate::Error> {
        let book = self.inner.host.load()?;
        let pending: Vec<String> = {
            let live = self.inner.state.lock().await;
            book.peers
                .iter()
                .map(|peer| peer.creds.desktop_id.clone())
                .filter(|desktop_id| !live.live.contains_key(desktop_id))
                .collect()
        };
        // In parallel: each attempt is a handshake with its own budget, so doing
        // them one after another would make "reconnect all" take as long as the sum
        // of the machines that happen to be asleep.
        futures::future::join_all(pending.into_iter().map(|desktop_id| {
            let emitter = emitter.clone();
            async move {
                if let Err(error) = self.connect_with_emitter(&desktop_id, emitter).await {
                    eprintln!("remote_peer: could not reach {desktop_id}: {error}");
                }
            }
        }))
        .await;
        self.list().await
    }
    /// Connect and, when an emitter is supplied, start streaming that host's events
    /// into it.
    pub async fn connect_with_emitter(
        &self,
        desktop_id: &str,
        emitter: Option<Emitter>,
    ) -> Result<PeerSummary, crate::Error> {
        self.start_rotation_worker();
        self.connect_attempt(desktop_id, emitter, true).await
    }
    async fn connect_attempt(
        &self,
        desktop_id: &str,
        emitter: Option<Emitter>,
        replace_supervisor: bool,
    ) -> Result<PeerSummary, crate::Error> {
        self.ensure_running()?;
        let epoch = self
            .begin_connect_attempt(desktop_id, replace_supervisor)
            .await?;
        let mut creds = self.stored_creds(desktop_id)?;
        // Renew before connecting: a token that expires mid-handshake fails it in a
        // way that looks like a protocol fault.
        if creds.needs_refresh(self.now_secs()) {
            match self.renew_credentials(&creds, epoch).await {
                Ok(renewed) => creds = renewed,
                Err(error) => {
                    let mut state = self.inner.state.lock().await;
                    if self.check_attempt(&state, desktop_id, epoch).is_ok() {
                        state
                            .errors
                            .insert(desktop_id.to_string(), error.to_string());
                    }
                    return Err(error);
                }
            }
        }
        {
            let state = self.inner.state.lock().await;
            self.check_attempt(&state, desktop_id, epoch)?;
        }
        let connected = match self
            .while_running(session::connect(
                &creds,
                &self.inner.host.connection_options(),
            ))
            .await
        {
            Ok(connected) => connected,
            Err(error) => {
                let mut live = self.inner.state.lock().await;
                self.check_attempt(&live, desktop_id, epoch)?;
                // A connect that failed for a *transient* reason — a socket that
                // timed out, a refused connection, DNS — leaves the pairing intact,
                // so the host is merely unreachable right now, and nothing but a user
                // action used to clear it. That is the connect-leg half of the shape
                // §13.7 fixed for requests (`docs/testing/flaky-tauri.md` §13.8).
                //
                // The caller still gets the error: a user who pressed Connect sees it.
                // What changes is that a *bounded* retry is also scheduled, so the
                // host comes back on its own instead of waiting for someone to press
                // the button again.
                let schedule_retry = should_retry_connect(
                    &error,
                    live.stopped.contains(desktop_id),
                    live.supervisors.contains_key(desktop_id),
                );
                if schedule_retry {
                    let supervisor = self.spawn_supervisor(desktop_id.to_string(), emitter.clone());
                    live.supervisors.insert(desktop_id.to_string(), supervisor);
                }
                live.errors
                    .insert(desktop_id.to_string(), error.to_string());
                return Err(error);
            }
        };
        // The PSK has now been spent. Drop it from disk while we know the host has
        // bound this identity; from here every connection is `Noise_IK`.
        {
            let state = self.inner.state.lock().await;
            self.check_attempt(&state, desktop_id, epoch)?;
            if connected.consumed_invitation {
                if let Err(error) = self.inner.host.clear_secret(&creds) {
                    eprintln!("remote_peer: could not clear the invitation secret: {error}");
                }
            }
        }
        let mut summary = self.paired_summary(desktop_id)?;
        summary.connected = true;
        // What the UI is looking at, so a reconnection comes back on the lane the
        // user left it on instead of re-subscribing the whole host. Read (and the
        // receiver taken) before the subscribe below, which is the only ordering
        // that makes the value the same one the subscription is built from. The lock
        // is short and not held across a network round-trip.
        //
        // The same lock resolves *where the pushes go*. A caller that supplied an
        // emitter is (re)stating the app's interest in this host, so it replaces
        // whatever was registered; a caller that did not — an internal
        // `ensure_connected`, the supervisor resuming a dropped link — gets the
        // registered one back. Without that second half, the first command to
        // reconnect a host after a drop left it connected and deaf: every command
        // answered and no push ever reached the UI again.
        let (visible_now, visible_changes, watcher) = {
            let mut live = self.inner.state.lock().await;
            self.check_attempt(&live, desktop_id, epoch)?;
            let watcher = match emitter {
                Some(emitter) => {
                    live.emitters
                        .insert(desktop_id.to_string(), emitter.clone());
                    Some(emitter)
                }
                None => live.emitters.get(desktop_id).cloned(),
            };
            // A connection the user asked for, or this app's own liveness pass
            // retrying a host that was only ever unreachable, is proof the host is
            // wanted live again.
            live.stopped.remove(desktop_id);
            let changes = live
                .visible
                .entry(desktop_id.to_string())
                .or_default()
                .clone();
            let now = changes.borrow().clone();
            (now, changes.subscribe(), watcher)
        };
        // Subscribe before taking the runtime lock: subscription is a network
        // round-trip, and a failure there must not keep a usable connection out of
        // the map.
        let peer_session = std::sync::Arc::new(connected.session);
        let stream = match watcher {
            Some(emitter) => {
                attach_stream(&peer_session, emitter, desktop_id, visible_now.as_deref()).await
            }
            None => None,
        };
        let previous = {
            let mut live = self.inner.state.lock().await;
            self.check_attempt(&live, desktop_id, epoch)?;
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
                    self.spawn_event_stream(
                        desktop_id.to_string(),
                        peer_session.pair_id().to_string(),
                        peer_session.event_lane(),
                        visible_now,
                        visible_changes,
                        parts,
                    ),
                );
            }
            let previous = live
                .live
                .insert(desktop_id.to_string(), peer_session.clone());
            // A live connection is proof the retry loop is no longer needed; it is
            // restarted below only if this connection later drops.
            if let Some(supervisor) = live.supervisors.remove(desktop_id) {
                supervisor.abort();
            }
            // Start proving this client is here, on the connection that was just
            // installed. A reconnect replaces it: the old task's session is no
            // longer the live one and exits on its next tick (`spawn_presence_beats`).
            let beats = self
                .spawn_presence_beats(desktop_id.to_string(), std::sync::Arc::clone(&peer_session));
            if let Some(previous) = live.beats.insert(desktop_id.to_string(), beats) {
                previous.abort();
            }
            previous
        };
        // The replacement is installed and ready, so the generation it replaced is
        // retired now — not merely forgotten. Its socket belongs to a grant this
        // one supersedes, and a socket nothing closes sits half-open (CLOSE_WAIT)
        // until the process ends.
        if let Some(previous) = previous {
            let _ = previous.close_socket().await;
        }
        Ok(summary)
    }

    /// Tell the host this client is here, for as long as this connection is the
    /// live one.
    ///
    /// The promise made by [`future_remote_protocol::CLIENT_PRESENCE_FEATURE`]:
    /// an idle client sends nothing, and a host that knows the capability reads
    /// that silence as a departure. Returns as soon as the session is no longer
    /// the live one, is stopped, or the socket refuses a beat — each of those has
    /// an owner elsewhere (a reconnect, the user's stop, the recovery path), and
    /// a beat task that outlived its connection would tell the host the wrong
    /// thing on the right pairing.
    fn spawn_presence_beats(
        &self,
        desktop_id: String,
        session: std::sync::Arc<PeerSession>,
    ) -> TaskHandle {
        let client = self.clone();
        self.spawn(async move {
            let mut beat = tokio::time::interval(presence_beat_interval());
            // A laptop that slept is not behind: skip the missed ticks instead
            // of firing a burst of them when it wakes.
            beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                beat.tick().await;
                {
                    let state = client.inner.state.lock().await;
                    if client.is_stopped()
                        || state.stopped.contains(&desktop_id)
                        || !state
                            .live
                            .get(&desktop_id)
                            .is_some_and(|current| std::sync::Arc::ptr_eq(current, &session))
                    {
                        return;
                    }
                }
                if session.beat_presence().await.is_err() {
                    // The transport owns the failure: a dropped socket is the
                    // supervisor's to rebuild, and a beat that could not leave
                    // costs at most the lease, which is the same answer a client
                    // that crashed gets.
                    return;
                }
            }
        })
    }

    /// Rotate a live generation's grant before the relay stops accepting it.
    ///
    /// A socket is authorized by the JWT it was opened with: once that grant
    /// expires the relay closes the socket, and `async-nats` can only reconnect
    /// with the same expired JWT. The phone client solves this the same way
    /// (`scheduleRefresh`): wake when the grant is due, refresh it, and rebuild
    /// the connection, so the generation that ends is the one being replaced
    /// rather than one the relay refused.
    ///
    /// One worker per client, started from the outermost connect entry points —
    /// never from `connect_attempt`, because the worker rotates *by* connecting
    /// and a start inside that path would make the two futures contain each
    /// other. The decision is [`PeerCreds::needs_refresh`] on the grant the store
    /// holds; the tick only bounds how often that grant is re-read.
    fn start_rotation_worker(&self) {
        if self
            .inner
            .rotation_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let client = self.clone();
        let handle = self.spawn(async move {
            loop {
                tokio::time::sleep(client.next_rotation_check().await).await;
                if client.is_stopped() {
                    return;
                }
                for desktop_id in client.hosts_due_for_rotation().await {
                    // Not fatal: the serving connection still works until its own
                    // expiry, and the bounded reconnect is the fallback if the
                    // grant lapses.
                    if let Err(error) = client.connect_attempt(&desktop_id, None, false).await {
                        eprintln!(
                            "remote_peer: could not rotate the credential for {desktop_id}: {error}"
                        );
                    }
                }
            }
        });
        // Registered with the client's own tasks, so shutdown waits for it; the
        // handle itself is not needed again.
        let _ = handle;
    }

    /// How long until the earliest live grant is due, bounded by the tick.
    ///
    /// The deadline is the credential's own: its declared expiry minus
    /// [`creds::REFRESH_LEAD_SECS`]. A due grant returns zero, so the next check
    /// rotates it immediately.
    async fn next_rotation_check(&self) -> Duration {
        let live: Vec<String> = self.inner.state.lock().await.live.keys().cloned().collect();
        // Nothing connected: nothing to rotate, and no reason to touch the
        // credential store at all.
        if live.is_empty() {
            return rotation_tick();
        }
        let now = self.now_secs();
        let Ok(book) = self.inner.host.load() else {
            return rotation_tick();
        };
        let rotated = self
            .inner
            .rotated_for
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        live.iter()
            .filter_map(|desktop_id| book.find(desktop_id).map(|record| (desktop_id, record)))
            .fold(rotation_tick(), |wait, (desktop_id, record)| {
                // A grant this worker already rotated for gets the tick: waking
                // for it again immediately would reconnect on every pass.
                if rotated.get(desktop_id) == Some(&record.creds.jwt_expires_at) {
                    return wait;
                }
                wait.min(rotation_wait(record.creds.jwt_expires_at, now))
            })
    }

    /// The live hosts whose stored grant has entered the refresh window.
    async fn hosts_due_for_rotation(&self) -> Vec<String> {
        let live: Vec<String> = self.inner.state.lock().await.live.keys().cloned().collect();
        if live.is_empty() {
            return Vec::new();
        }
        let now = self.now_secs();
        let Ok(book) = self.inner.host.load() else {
            return Vec::new();
        };
        let due: Vec<String> = live
            .into_iter()
            .filter(|desktop_id| {
                book.find(desktop_id)
                    .is_some_and(|record| record.creds.needs_refresh(now))
            })
            .collect();
        let mut rotated = self
            .inner
            .rotated_for
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        due.into_iter()
            .filter(|desktop_id| {
                // One rotation per grant: see `next_rotation_check`.
                let Some(record) = book.find(desktop_id) else {
                    return false;
                };
                if rotated.get(desktop_id) == Some(&record.creds.jwt_expires_at) {
                    return false;
                }
                rotated.insert(desktop_id.clone(), record.creds.jwt_expires_at);
                true
            })
            .collect()
    }

    /// The generation that owned the host is over: its socket closed for good.
    ///
    /// Called by the event stream when every subscription ended, which
    /// `async-nats` only does once its reconnect budget is spent — a refused
    /// credential, or a socket that never came back. Until this is recorded the
    /// runtime keeps reporting `connected: true` for a socket that can send
    /// nothing, and no retry ever runs.
    async fn generation_ended(&self, desktop_id: &str, channel: &SessionChannel) {
        let (ended, stopped) = {
            let mut live = self.inner.state.lock().await;
            let current = live.live.get(desktop_id).is_some_and(|session| {
                std::sync::Arc::ptr_eq(&session.channel_for_stream(), channel)
            });
            if !current {
                // A newer generation owns the host, or the host was stopped: this
                // task is being retired, and the connection is not to blame.
                return;
            }
            live.errors.insert(
                desktop_id.to_string(),
                "nats_connection_exhausted".to_string(),
            );
            let ended = live.live.remove(desktop_id);
            (ended, live.stopped.contains(desktop_id))
        };
        if let Some(session) = ended {
            let _ = session.close_socket().await;
        }
        if stopped || self.is_stopped() {
            return;
        }
        // The same bounded recovery a transport failure uses. That path renews the
        // grant first (`connect_attempt`), which is the one thing a library-level
        // reconnect loop can never do.
        let mut live = self.inner.state.lock().await;
        if !live.supervisors.contains_key(desktop_id) {
            let supervisor = self.spawn_supervisor(desktop_id.to_string(), None);
            live.supervisors.insert(desktop_id.to_string(), supervisor);
        }
    }
    ///
    /// Keep one host connected: watch the socket, and reconnect when it drops.
    ///
    /// Runs until it succeeds, until its retry window is exhausted, or until it is
    /// aborted (an explicit disconnect, or a newer connection replacing it).
    ///
    /// Also a process-lifetime task, and therefore spawned through
    /// the host task registry for the same reason as the stream above: a retry loop that
    /// outlives its test would keep reconnecting under the next test's `HOME`.
    pub fn spawn_supervisor(&self, desktop_id: String, emitter: Option<Emitter>) -> TaskHandle {
        let client = self.clone();
        self.spawn(async move {
            let mut attempt = 0usize;
            loop {
                tokio::time::sleep(reconnect_delay(attempt)).await;
                if client.is_stopped()
                    || client
                        .inner
                        .state
                        .lock()
                        .await
                        .stopped
                        .contains(&desktop_id)
                {
                    return;
                }
                // Already connected (something else reconnected while we waited).
                let connected = client
                    .inner
                    .state
                    .lock()
                    .await
                    .live
                    .contains_key(&desktop_id);
                if connected {
                    return;
                }
                // Where this host's pushes go, resolved per attempt: a caller may
                // hand one in, but the loop's ordinary callers have none of their
                // own, and the emitter the app registered is what keeps a recovered
                // host *delivering* rather than merely answering. Without it the
                // reconnect produced a live, watched-looking connection that pushed
                // nothing at all 鈥?a dead live lane the UI only noticed when
                // something else happened to re-read.
                let watcher = match &emitter {
                    Some(emitter) => Some(emitter.clone()),
                    None => client
                        .inner
                        .state
                        .lock()
                        .await
                        .emitters
                        .get(&desktop_id)
                        .cloned(),
                };
                // One retry is being dispatched: this is the seam the budget
                // counts. Recorded before the attempt, so a connect still in
                // flight is already accounted for rather than only once it fails.
                #[cfg(any(test, feature = "test-support"))]
                client.note_retry_attempt(&desktop_id).await;
                match client.resume(&desktop_id, watcher.clone()).await {
                    Ok(()) => {
                        // Say so, and say it *after* the subscriptions are attached:
                        // a client told to catch up before the stream is live can
                        // still miss the events that arrive in between.
                        if let Some(emitter) = &watcher {
                            emitter(PeerEvent {
                                desktop_id: desktop_id.clone(),
                                kind: "resumed",
                                payload: json!({}),
                            });
                        }
                        return;
                    }
                    Err(error) => {
                        eprintln!(
                            "remote_peer: reconnect attempt for {desktop_id} failed: {error}"
                        );
                        // A refused credential is not a network fault. The same
                        // grant cannot authenticate however often it is presented,
                        // and retrying it is exactly the loop this supervisor was
                        // bounded to avoid; the rotation worker (which renews) or a
                        // user action is what can change it. The error is already
                        // recorded by the attempt.
                        if matches!(error, crate::Error::RemoteAuthorization(_)) {
                            return;
                        }
                    }
                }
                attempt += 1;
                // Measured in *attempts*, not wall clock: the delay schedule already
                // caps the wait, and a clock here would need a monotonic source the
                // tests would have to fake.
                if attempt >= reconnect_attempt_budget() {
                    // Network outages are not crash loops. Keep one cancellable
                    // owner asleep, then retry; never resend business commands.
                    tokio::time::sleep(delay_unit() * 300).await;
                    attempt = 0;
                }
            }
        })
    }
    /// Refresh credentials when needed, then reconnect and re-subscribe.
    async fn resume(&self, desktop_id: &str, emitter: Option<Emitter>) -> Result<(), crate::Error> {
        self.start_rotation_worker();
        self.connect_attempt(desktop_id, emitter, false)
            .await
            .map(|_| ())
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
    /// Spawned through the host task registry rather than `tokio::spawn`: this is a
    /// process-lifetime task, and the crate's spawner is what lets a test fixture
    /// stop it before switching `HOME`. A raw spawn outlives the runtime that
    /// started it — in the test harness that means one test's stream still running
    /// while another test owns the process-global runtime and `HOME`.
    fn spawn_event_stream(
        &self,
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
    ) -> TaskHandle {
        self.spawn_event_stream_capped(
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
    #[allow(clippy::too_many_arguments)]
    fn spawn_event_stream_capped(
        &self,
        desktop_id: String,
        pair_id: String,
        lane: super::session::EventLane,
        subscribed_with: Option<String>,
        visible: tokio::sync::watch::Receiver<Option<String>>,
        parts: StreamParts,
        backlog_capacity: usize,
    ) -> TaskHandle {
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
            control,
        } = parts;
        let client = self.clone();
        self.spawn(async move {
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
            #[allow(clippy::too_many_arguments)]
            async fn handle_push(
                client: &Client,
                push: Push,
                channel: &std::sync::Mutex<future_remote_crypto::Channel>,
                control: &std::sync::Mutex<session::Control>,
                desktop_id: &str,
                pair_id: &str,
                narrow: bool,
                emitter: &Emitter,
            ) {
                {
                    let state = client.inner.state.lock().await;
                    if client.is_stopped() || state.stopped.contains(desktop_id) {
                        return;
                    }
                    if !state.live.get(desktop_id).is_some_and(|current| {
                        std::ptr::eq(Arc::as_ptr(&current.channel_for_stream()), channel)
                    }) {
                        return;
                    }
                }
                let Push { kind, message } = push;
                if kind == TRANSFER_LANE {
                    // Parsed before decrypting: a malformed subject is dropped
                    // without spending a decryption (and the traffic-key lock) on
                    // it, and a relay that spams the transfer prefix cannot make
                    // this client work to throw the messages away.
                    let Some((transfer_id, index)) = parse_chunk_subject(&message.subject) else {
                        return;
                    };
                    let Some(bytes) = open_bytes(channel, &message.subject, &message.payload)
                    else {
                        // Unauthenticated: a relay can inject one at will, and the
                        // connection is not at fault, so drop it.
                        return;
                    };
                    client
                        .deliver_chunk(desktop_id, transfer_id, index, bytes)
                        .await;
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
                        StateEventKind::Activity => (CATALOG_LANE, Some("sessions_activity")),
                        StateEventKind::Workspaces => (CATALOG_LANE, Some("workspaces")),
                        StateEventKind::Control => (CONTROL_LANE, None),
                        _ => return,
                    }
                } else {
                    (kind, None)
                };
                let Some(mut payload) = open(channel, &message.subject, &message.payload) else {
                    // Unparsable or unauthenticated; same reasoning as above.
                    return;
                };
                let mut agent_recovered = false;
                if kind == PRESENCE_LANE || kind == CONTROL_LANE {
                    let mut held = control
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if kind == PRESENCE_LANE {
                        agent_recovered = held.agent_available == Some(false)
                            && payload.get("agentAvailable").and_then(Value::as_bool) == Some(true);
                        held.apply_presence(&payload);
                    } else {
                        held.apply_control(&payload);
                    }
                }
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
                if agent_recovered {
                    // The bridge stayed authenticated while its Agent restarted.
                    // Pull authoritative state, never re-submit the previous run.
                    emitter(PeerEvent {
                        desktop_id: desktop_id.to_string(),
                        kind: "resumed",
                        payload: json!({}),
                    });
                }
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
                            // Every stream ended: this socket is closed for good.
                            // Retire the generation unless it has already been
                            // replaced, so a dead connection cannot keep being
                            // reported as live and the bounded supervisor
                            // reconnects with a renewed grant.
                            client.generation_ended(&desktop_id, &channel).await;
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
                        &client,
                        push,
                        &channel,
                        &control,
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
    /// Hand one decrypted chunk to whoever is waiting for it.
    ///
    /// A chunk nobody is waiting for is dropped rather than queued: the host only
    /// sends bytes in answer to a pull, so an unmatched one is either a duplicate
    /// from a retry or a reply to a pull that already timed out. Buffering it would
    /// keep file bytes in memory for a download the caller has abandoned.
    async fn deliver_chunk(
        &self,
        desktop_id: &str,
        transfer_id: String,
        index: u64,
        bytes: Vec<u8>,
    ) {
        let waiter = self
            .inner
            .chunks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(desktop_id.to_string(), transfer_id, index));
        if let Some(waiter) = waiter {
            // A closed receiver means the pull gave up; the bytes are then unwanted.
            let _ = waiter.sender.send(bytes);
        }
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
    pub async fn set_visible_session(&self, desktop_id: &str, session_id: Option<&str>) {
        let mut live = self.inner.state.lock().await;
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
    ///
    /// Records the host as *stopped*, which is what keeps a deliberate stop from
    /// being undone by the app's own liveness pass: that pass reconnects hosts that
    /// dropped, and without this it could not tell one the user turned off from one
    /// that fell over. Connecting the host later clears the mark.
    pub async fn disconnect(&self, desktop_id: &str) {
        // Tell the host this was the user's stop, not a crash — *before* the
        // connection is torn down, while the socket that carries it is still
        // the live one. Without it the host keeps a bridge pointed at a client
        // that is gone and its Remote page goes on saying "in control" until
        // the lease lapses, which is the exact stale row a disconnect is
        // supposed to clear. Bounded by the goodbye's own budget
        // (`PeerSession::leave`); the lease is the fallback.
        let session = self.session_handle(desktop_id).await;
        if let Some(session) = &session {
            if let Err(error) = session.leave("disconnected").await {
                eprintln!("remote_peer: could not tell {desktop_id} we left: {error}");
            }
        }
        {
            let mut live = self.inner.state.lock().await;
            let epoch = live.epochs.entry(desktop_id.to_string()).or_default();
            *epoch = epoch.wrapping_add(1);
            self.inner
                .chunks
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|(desktop, _, _), _| desktop != desktop_id);
            if let Some(task) = live.tasks.remove(desktop_id) {
                task.abort();
            }
            if let Some(beats) = live.beats.remove(desktop_id) {
                beats.abort();
            }
            // Without this, a retry scheduled a second ago would silently bring the
            // connection back after the user asked for it to stop.
            if let Some(supervisor) = live.supervisors.remove(desktop_id) {
                supervisor.abort();
            }
            live.stopped.insert(desktop_id.to_string());
            live.live.remove(desktop_id);
        }
        if let Some(session) = session {
            let _ =
                tokio::time::timeout(session::PRESENCE_CLOSE_BUDGET, session.close_socket()).await;
        }
    }
    /// Drop the pairing locally, then try to revoke it server-side.
    ///
    /// Local first, always: an unreachable platform must not leave a host the user
    /// removed still connectable. A revoke failure is *returned* rather than
    /// swallowed so a caller can queue a retry, but it is not fatal — pairing again
    /// is a new invitation either way.
    pub async fn unpair(&self, desktop_id: &str) -> Result<Option<String>, crate::Error> {
        let creds = self.stored_creds(desktop_id)?;
        self.disconnect(desktop_id).await;
        if !self.inner.host.remove(&creds)? {
            return Err(crate::Error::Message("peer_not_paired".into()));
        }
        {
            // The pairing is gone, so nothing should remember where its pushes
            // went or that the user had stopped it: re-pairing is a new host as far
            // as this state is concerned.
            let mut live = self.inner.state.lock().await;
            live.emitters.remove(desktop_id);
            live.stopped.remove(desktop_id);
            live.errors.remove(desktop_id);
        }
        Ok(self
            .inner
            .host
            .revoke(&creds)
            .await
            .err()
            .map(|error| error.to_string()))
    }
    pub fn set_label(
        &self,
        desktop_id: &str,
        name: Option<&str>,
        icon: Option<&str>,
    ) -> Result<(), crate::Error> {
        self.inner.host.set_label(desktop_id, name, icon)
    }
    /// The remote catalogue, stamped with the host it came from.
    ///
    /// The stamp is added here rather than derived by the caller: a merged list is
    /// built from several hosts' snapshots, and a row that cannot name its host
    /// would be routed to the wrong machine on click.
    pub async fn sessions(&self, desktop_id: &str) -> Result<Value, crate::Error> {
        let data = self
            .request(desktop_id, json!({ "type": "list_sessions" }), "list")
            .await?;
        self.stamp_live(desktop_id, data).await
    }
    pub async fn workspaces(&self, desktop_id: &str) -> Result<Value, crate::Error> {
        let data = self
            .request(desktop_id, json!({ "type": "list_workspaces" }), "list")
            .await?;
        self.stamp_live(desktop_id, data).await
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
    pub async fn request(
        &self,
        desktop_id: &str,
        command: Value,
        lane: &str,
    ) -> Result<Value, crate::Error> {
        self.request_bound(desktop_id, command, lane, None).await
    }
    pub(crate) async fn request_for_transfer(
        &self,
        desktop_id: &str,
        epoch: u64,
        command: Value,
        lane: &str,
    ) -> Result<Value, crate::Error> {
        self.request_bound(desktop_id, command, lane, Some(epoch))
            .await
    }
    async fn request_bound(
        &self,
        desktop_id: &str,
        command: Value,
        lane: &str,
        expected: Option<u64>,
    ) -> Result<Value, crate::Error> {
        // Read before the command is consumed: the chunk fetches have to name the
        // same session and run as the page they are completing.
        let owner = ChunkOwner::from(&command);
        let reply = self
            .send_request_bound(desktop_id, command, lane, expected)
            .await?;
        match read_chunk_of(&reply) {
            // A chunked reply: put the page back together before returning it, so no
            // caller has to know a page can arrive in pieces.
            Some(chunk) => {
                self.reassemble(desktop_id, lane, chunk, &owner, expected)
                    .await
            }
            None => Ok(reply),
        }
    }
    /// One round trip: the command and its reply, unreassembled.
    ///
    /// Split from [`request`] because chunk fetching goes through it directly: a
    /// `get_read_chunk` reply *is* a chunk, and routing it back through the
    /// reassembling wrapper would make the two functions mutually recursive.
    async fn send_request_bound(
        &self,
        desktop_id: &str,
        mut command: Value,
        lane: &str,
        expected: Option<u64>,
    ) -> Result<Value, crate::Error> {
        if expected.is_none() {
            self.ensure_connected(desktop_id).await?;
        }
        if let Some(object) = command.as_object_mut() {
            object
                .entry("id")
                .or_insert_with(|| json!(self.inner.host.command_id()));
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
        // The handle is cloned out of the map, not borrowed from it: the round trip
        // below is a network wait, and the runtime mutex is what every other host's
        // `list()`, `connect` and `disconnect` needs. This is the second half of
        // why a session is an `Arc` — the wait happens with the lock released.
        let bound = if let Some(expected) = expected {
            let (epoch, session) = self.transfer_session(desktop_id).await?;
            if epoch != expected {
                return Err(crate::Error::Message("peer_not_connected".into()));
            }
            Some(session)
        } else {
            self.session_handle(desktop_id).await
        };
        let Some(session) = bound else {
            return Err(crate::Error::Message("peer_not_connected".into()));
        };
        let subject = session.command_subject(&key);
        let result = self
            .while_running(session.request(&subject, command, session::COMMAND_TIMEOUT))
            .await;
        if let Err(error) = &result {
            let mut live = self.inner.state.lock().await;
            if live
                .live
                .get(desktop_id)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                self.on_transport_failure(&mut live, desktop_id, error);
            }
        }
        result
    }
    /// Fetch the rest of a page's chunks and decode it.
    async fn reassemble(
        &self,
        desktop_id: &str,
        lane: &str,
        first: &Value,
        owner: &ChunkOwner,
        expected: Option<u64>,
    ) -> Result<Value, crate::Error> {
        let id = chunk_string(first, "id")?;
        let total = chunk_u64(first, "totalBytes")?;
        if total > MAX_PAGE_BYTES {
            return Err(crate::Error::Message("remote_read_too_large".into()));
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
            let fetched = futures::future::try_join_all(offsets.into_iter().map(|offset| {
                let id = &id;
                async move {
                    Ok::<_, crate::Error>((
                        offset,
                        self.fetch_chunk(desktop_id, lane, id, offset, owner, expected)
                            .await?,
                    ))
                }
            }))
            .await?;
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
        &self,
        desktop_id: &str,
        lane: &str,
        id: &str,
        offset: usize,
        owner: &ChunkOwner,
        expected: Option<u64>,
    ) -> Result<Value, crate::Error> {
        let mut command = json!({"type": "get_read_chunk", "replyId": id, "offset": offset});
        if let Some(session) = &owner.session {
            command["sessionId"] = json!(session);
        }
        if let Some(run) = &owner.run {
            command["runId"] = json!(run);
        }
        self.send_request_bound(desktop_id, command, lane, expected)
            .await
    }
    fn on_transport_failure(&self, live: &mut Runtime, desktop_id: &str, error: &crate::Error) {
        // Only a transport fault means the socket is gone. A host that answered and
        // refused is still connected.
        if !matches!(error, crate::Error::RemoteTransport(_)) {
            return;
        }
        // A *request* that timed out is not evidence that the connection died: a slow
        // host and a dead one are indistinguishable from the caller's seat, and
        // inferring death from one slow round trip is how a busy host became
        // `peer_not_connected` — the shape CI reports (`docs/testing/flaky-tauri.md`
        // §13.6.6, the `#1094` log). So ask the only component that knows: the NATS
        // client's own state, which its PING/PONG bookkeeping maintains.
        //
        // Measured, both arms: a socket closed under the connection yields a timeout
        // *and* `is_connected() == false` (so it is still dropped here, and the
        // retry loop still starts), while a starved but live peer yields a timeout
        // over a `Connected` client (so the connection — and its subscriptions — is
        // kept, and the failure stays the command's own).
        if is_request_timeout(error)
            && live
                .live
                .get(desktop_id)
                .is_some_and(|session| session.is_connected())
        {
            return;
        }
        // A dead socket must not keep looking connected on the next poll.
        live.errors
            .insert(desktop_id.to_string(), error.to_string());
        if let Some(session) = live.live.remove(desktop_id) {
            // Close the socket this failure condemned. Dropping the last client
            // clone usually closes it, but an in-flight command or a task that has
            // not been polled since still holds one, and an abandoned socket sits
            // in CLOSE_WAIT until the process ends.
            self.spawn(async move {
                let _ = session.close_socket().await;
            });
        }
        // The connection was healthy and then was not: exactly the case automatic
        // recovery is for. An explicit disconnect takes the supervisor with it, so
        // this cannot resurrect a stopped link.
        //
        // No emitter is handed over: the loop takes the one the app registered for
        // this host, which is the only way a *recovered* host still delivers what
        // it pushes (see `spawn_supervisor`).
        if !live.supervisors.contains_key(desktop_id) {
            let supervisor = self.spawn_supervisor(desktop_id.to_string(), None);
            live.supervisors.insert(desktop_id.to_string(), supervisor);
        }
    }
    /// Record one chunk of an upload.
    ///
    /// The mirror of `pull_chunk`, and simpler because there is nothing to join: the
    /// chunk *is* the request body, and the reply is the host's acknowledgement.
    pub async fn put_chunk(
        &self,
        desktop_id: &str,
        upload_id: &str,
        index: u64,
        bytes: &[u8],
    ) -> Result<(), crate::Error> {
        self.put_chunk_within(
            desktop_id,
            upload_id,
            index,
            bytes,
            session::COMMAND_TIMEOUT,
        )
        .await
    }
    /// [`put_chunk`], with the budget for the chunk write as an argument.
    ///
    /// The budget is a parameter for the same reason [`pull_chunk_within`]'s is: the
    /// one test that *wants* the give-up arm (a socket that is already closed) should
    /// not have to wait out the budget production gives a real host — and under
    /// `cfg(test)` that budget is the fixture's tolerance (see
    /// `session::COMMAND_TIMEOUT`), which would make "it failed" and "it took a
    /// minute" the same test.
    pub async fn put_chunk_within(
        &self,
        desktop_id: &str,
        upload_id: &str,
        index: u64,
        bytes: &[u8],
        budget: Duration,
    ) -> Result<(), crate::Error> {
        let session = self.inner.state.lock().await.live.get(desktop_id).cloned();
        let Some(session) = session else {
            return Err(crate::Error::Message("peer_not_connected".into()));
        };
        let subject = session.transfer_chunk_subject(upload_id, index);
        // The session budget, not a transfer one: the host writes this chunk to disk
        // before it answers, so a short budget here would report a slow disk as a
        // dead socket. The wait happens with the runtime lock released, so a slow
        // disk on this host does not stall another host's list read.
        let result = self
            .while_running(session.request_bytes(&subject, bytes, budget))
            .await;
        if let Err(error) = &result {
            let mut live = self.inner.state.lock().await;
            if live
                .live
                .get(desktop_id)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                self.on_transport_failure(&mut live, desktop_id, error);
            }
        }
        result.map(|_| ())
    }
    /// Where a chunk pull is requested. Built from the live session so the subject
    /// matches *this* pairing rather than whatever the credential file says.
    pub async fn transfer_pull_subject(
        &self,
        desktop_id: &str,
        transfer_id: &str,
        index: u64,
    ) -> Result<String, crate::Error> {
        let live = self.inner.state.lock().await;
        let Some(session) = live.live.get(desktop_id) else {
            return Err(crate::Error::Message("peer_not_connected".into()));
        };
        Ok(session.transfer_pull_subject(transfer_id, index))
    }
    /// Hold a place for the bytes of one chunk.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn register_chunk(
        &self,
        desktop_id: &str,
        transfer_id: &str,
        index: u64,
        sender: tokio::sync::oneshot::Sender<Vec<u8>>,
    ) {
        let key = (desktop_id.to_string(), transfer_id.to_string(), index);
        self.inner
            .chunks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                key,
                ChunkWaiter {
                    registration: Arc::new(()),
                    sender,
                },
            );
    }
    pub(crate) fn register_transfer_chunk(
        &self,
        desktop: &str,
        transfer: &str,
        index: u64,
        sender: tokio::sync::oneshot::Sender<Vec<u8>>,
    ) -> ChunkRegistration {
        let key = (desktop.to_string(), transfer.to_string(), index);
        let registration = Arc::new(());
        self.inner
            .chunks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                key.clone(),
                ChunkWaiter {
                    registration: registration.clone(),
                    sender,
                },
            );
        ChunkRegistration {
            client: self.clone(),
            key,
            registration,
        }
    }
    /// Give up a place for one chunk.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn forget_chunk(&self, desktop_id: &str, transfer_id: &str, index: u64) {
        self.inner
            .chunks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(desktop_id.to_string(), transfer_id.to_string(), index));
    }
    /// The bytes of one chunk: ask, and join the two halves of the answer.
    pub async fn pull_chunk(
        &self,
        desktop_id: &str,
        transfer_id: &str,
        index: u64,
    ) -> Result<Vec<u8>, crate::Error> {
        self.pull_chunk_within(desktop_id, transfer_id, index, chunk_pull_timeout())
            .await
    }
    /// [`pull_chunk`], with the wait for the bytes as an argument.
    ///
    /// The bytes arrive as a publish on a different subject from the pull's
    /// acknowledgement, so this is the only place the two are brought together —
    /// which is also why the waiter is registered *before* the request: the host
    /// publishes the chunk first, and a waiter added afterwards would miss it.
    ///
    /// The budget is a parameter so that the one test which wants to observe the
    /// give-up arm can pass its own, instead of the whole suite running on a
    /// shortened one: production and the suite share [`chunk_pull_timeout`], and
    /// that wait is the only wall clock on this path. A test-only 500 ms budget was
    /// measured reporting a *lost* publish for a chunk a loaded host had served
    /// (FLAKES.md item 8) — the arm below cannot tell "never published" from "not
    /// yet published", so a budget that only exists under `cfg(test)` decides the
    /// test's outcome by host scheduling.
    pub async fn pull_chunk_within(
        &self,
        desktop_id: &str,
        transfer_id: &str,
        index: u64,
        budget: Duration,
    ) -> Result<Vec<u8>, crate::Error> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let _registration = self.register_transfer_chunk(desktop_id, transfer_id, index, sender);
        let subject = match self
            .transfer_pull_subject(desktop_id, transfer_id, index)
            .await
        {
            Ok(subject) => subject,
            Err(error) => {
                return Err(error);
            }
        };
        // The *request* keeps its own budget, which is separate from `budget` above
        // and is the one that pays for the work: it makes the host read the chunk
        // off disk, publish it and flush. A short budget here fails the command,
        // which the transport layer reads as a dead socket: the connection is
        // dropped and the retry reports `peer_not_connected`, i.e. a slow disk on a
        // busy machine looks exactly like a network fault. `budget` above is only
        // the wait for the bytes, which the host has already published by the time
        // it acknowledges this request.
        self.request_at(desktop_id, json!({}), &subject, session::COMMAND_TIMEOUT)
            .await?;
        match tokio::time::timeout(budget, receiver).await {
            Ok(Ok(bytes)) => Ok(bytes),
            // Nothing arrived: either the sender was dropped (the connection went
            // away, or the runtime was cleared) or the host acknowledged a publish
            // that never reached this subscription. The two are indistinguishable
            // from here and both are transient, so they share one message — and the
            // caller retries either way.
            _ => Err(crate::Error::RemoteTransport(
                "remote_download_chunk_missing".into(),
            )),
        }
    }
    /// One command on an explicit subject, for the routes that are not the
    /// session-keyed command lane (file transfer addresses a transfer id).
    pub async fn request_at(
        &self,
        desktop_id: &str,
        mut command: Value,
        subject: &str,
        timeout: Duration,
    ) -> Result<Value, crate::Error> {
        self.ensure_connected(desktop_id).await?;
        if let Some(object) = command.as_object_mut() {
            object
                .entry("id")
                .or_insert_with(|| json!(self.inner.host.command_id()));
        }
        // Same shape as the session-keyed lane: take the handle, drop the runtime
        // lock, then wait. A chunk pull's command can take the host a while (its
        // budget is the session's), and every other host's state read needs this
        // mutex.
        let Some(session) = self.session_handle(desktop_id).await else {
            return Err(crate::Error::Message("peer_not_connected".into()));
        };
        let result = self
            .while_running(session.request(subject, command, timeout))
            .await;
        if let Err(error) = &result {
            let mut live = self.inner.state.lock().await;
            if live
                .live
                .get(desktop_id)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                self.on_transport_failure(&mut live, desktop_id, error);
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
    async fn stamp_live(&self, desktop_id: &str, data: Value) -> Result<Value, crate::Error> {
        let pair_id = self
            .inner
            .state
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
            .ok_or_else(|| crate::Error::Message("peer_reply_not_an_object".into()))?;
        object.insert("desktopId".into(), json!(desktop_id));
        // `null` rather than omitted when the connection has already gone: the row
        // still came from this host, and a reader can tell "no live pairing" from
        // "field missing".
        object.insert("pairId".into(), pair_id.map_or(Value::Null, |id| json!(id)));
        Ok(data)
    }
    fn stored_creds(&self, desktop_id: &str) -> Result<PeerCreds, crate::Error> {
        self.inner
            .host
            .load()?
            .find(desktop_id)
            .map(|peer| peer.creds.clone())
            .ok_or_else(|| crate::Error::Message("peer_not_paired".into()))
    }
    fn now_secs(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn the_attempt_budget_is_bounded() {
        assert_eq!(reconnect_attempt_budget(), RECONNECT_DELAYS_SECS.len() * 3);
        assert!(reconnect_attempt_budget() > 0);
    }
    /// The rotation deadline is the grant's own expiry, not a fixed cadence.
    ///
    /// `now` is chosen far from any boundary so the arithmetic is the assertion:
    /// a grant that expires in an hour is not due for 59 minutes; one that is
    /// already inside the lead window is due *now*; and one with no declared
    /// expiry has nothing to schedule.
    #[test]
    fn a_grant_is_rotated_at_its_own_deadline() {
        let now = 1_000_000;
        let tick = rotation_tick();
        // 3600 s left: due after 3600 - lead, which the tick caps.
        assert_eq!(
            rotation_wait(now + 3_600, now),
            tick.min(std::time::Duration::from_secs(
                (3_600 - creds::REFRESH_LEAD_SECS) as u64
            ))
        );
        // Exactly at the lead: due now.
        assert_eq!(
            rotation_wait(now + creds::REFRESH_LEAD_SECS, now),
            std::time::Duration::ZERO
        );
        // Inside the lead, or already past it: due now (never a negative wait).
        assert_eq!(
            rotation_wait(now + creds::REFRESH_LEAD_SECS - 1, now),
            std::time::Duration::ZERO
        );
        assert_eq!(rotation_wait(now - 10, now), std::time::Duration::ZERO);
        // No declared expiry: only the tick re-reads it.
        assert_eq!(rotation_wait(0, now), tick);
    }
    /// Both rotation sides read the *same* predicate, so a grant cannot be
    /// rotated and then immediately treated as still needing rotation.
    #[test]
    fn the_refresh_window_has_one_definition() {
        let mut creds = PeerCreds {
            pair_id: "pair_1".into(),
            desktop_id: "desktop_1".into(),
            device_id: "device_1".into(),
            nkey_seed: String::new(),
            user_jwt: String::new(),
            refresh_token: String::new(),
            nats_url: String::new(),
            nats_ws_url: String::new(),
            jwt_expires_at: 1_000,
            token_url: String::new(),
            secure: None,
        };
        assert!(!creds.needs_refresh(1_000 - creds::REFRESH_LEAD_SECS - 1));
        assert!(creds.needs_refresh(1_000 - creds::REFRESH_LEAD_SECS));
        assert_eq!(
            creds::REFRESH_LEAD_SECS,
            60,
            "the margin is the phone client's; moving it moves both clients"
        );
        creds.jwt_expires_at = 0;
        assert!(
            !creds.needs_refresh(i64::MAX),
            "a grant with no declared expiry is never due"
        );
    }
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
        // The activity packet is a catalog update too, and it is *not* a
        // snapshot: a reader that confused the two would read its missing rows
        // as a deletion.
        assert!(matches!(
            state_event_kind("p.pair_1.state.activity", "pair_1"),
            StateEventKind::Activity
        ));
        assert!(matches!(
            state_event_kind("p.pair_1.state.control", "pair_1"),
            StateEventKind::Control
        ));
        for subject in [
            "p.pair_1.state.version",
            // Another pairing's host-wide lane.
            "p.pair_10.state.events",
            "p.pair_10.state.activity",
            "p.pair_10.state.control",
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
    #[test]
    fn the_batch_ceilings_are_the_mobile_clients() {
        assert_eq!(EVENT_BATCH_MAX, 64);
        assert_eq!(EVENT_BATCH_BYTES, 256 * 1024);
        assert_eq!(EVENT_BATCH_DEADLINE, std::time::Duration::from_millis(8));
        assert_eq!(EVENT_BACKLOG_CAPACITY, 4096);
    }
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
    #[test]
    fn a_zero_capacity_backlog_still_holds_one() {
        let mut backlog = Backlog::new(0);
        assert!(backlog.push("a"));
        assert!(!backlog.push("b"));
        assert_eq!(backlog.pop_front(), Some("a"));
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use super::*;
    pub const BACKLOG_CAPACITY: usize = EVENT_BACKLOG_CAPACITY;
    pub async fn visibility_change(
        visible: &mut tokio::sync::watch::Receiver<Option<String>>,
    ) -> Option<String> {
        super::visibility_change(visible).await
    }
    pub async fn apply_visible_lane(
        lane: &session::EventLane,
        installed: &mut session::EventsSubscription,
        events: &mut async_nats::Subscriber,
        visible: Option<&str>,
        desktop: &str,
    ) {
        super::apply_visible_lane(lane, installed, events, visible, desktop).await
    }
}

#[cfg(test)]
#[path = "runtime_ownership_tests.rs"]
mod ownership_tests;

#[cfg(test)]
#[path = "runtime_policy_tests.rs"]
mod policy_tests;
