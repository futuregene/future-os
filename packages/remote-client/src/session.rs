//! The live connection to one remote host: NATS socket, Noise handshake, and
//! the encrypted request/reply used for every command.
//!
//! This is the client mirror of `remote::commands` + `remote::secure`. The
//! shapes are fixed by the host and shared with the phone, so this module is
//! where "the desktop must look exactly like a phone" is enforced:
//!
//! * commands go to `p.{pair}.cmd.{sessionKey}` and are sealed with the subject
//!   as associated data;
//! * a reply is sealed against `reply:{subject}:{header}`, derived from the
//!   request bytes — a reply cannot be moved to another subject or replayed
//!   onto another request;
//! * the handshake pins the host's Noise key **before** any traffic: the key
//!   arrives in the invitation, so the first handshake is authenticated by the
//!   QR itself rather than by trust-on-first-use;
//! * a command whose reply is lost is re-sent — up to three attempts, 250·2ⁿ ms
//!   apart, always under the same command `id` — so the host's own reply cache
//!   answers the retry instead of running the command a second time.
//!
//! Beyond that command bound, nothing here decides *policy* (when to connect,
//! when to give up, what to do with an event). It performs one connection
//! attempt and exposes it.

use super::credentials::PeerCreds;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

/// Environment supplied by the host for one connection attempt. Platform
/// pairings require TLS; a declared plaintext relay retains its token-URL policy.
/// The host chooses the device labels and trust bundle.
#[derive(Clone, Debug)]
pub struct ConnectionOptions {
    pub device_name: String,
    pub device_kind: String,
    pub root_bundle: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    loopback_fixture: bool,
}

impl ConnectionOptions {
    pub fn new(device_name: impl Into<String>, device_kind: impl Into<String>) -> Self {
        Self {
            device_name: device_name.into(),
            device_kind: device_kind.into(),
            root_bundle: None,
            #[cfg(any(test, feature = "test-support"))]
            loopback_fixture: false,
        }
    }

    /// Only test builds can opt out of TLS, and connect additionally checks that
    /// the destination is loopback before opening the fixture transport.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_loopback_fixture(mut self) -> Self {
        self.loopback_fixture = true;
        self
    }

    fn insecure_loopback(&self) -> bool {
        #[cfg(any(test, feature = "test-support"))]
        return self.loopback_fixture;
        #[cfg(not(any(test, feature = "test-support")))]
        false
    }
}

/// The phone's budget for one command or handshake request.
///
/// `mobile/src/remote/client.ts` passes `{ timeout: 10_000 }` on both lanes, so
/// this is the client contract rather than a local choice. It is a *named*
/// constant so it can be pinned by a test and so the fixture tolerance below
/// has something to be a tolerance **of**.
const PHONE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the socket-connect leg gets.
///
/// This is `async-nats`'s own default for `ConnectOptions::connection_timeout`
/// (0.49: 5 s). It is written down and set explicitly rather than inherited:
/// inherited, a dependency bump could silently change how long the app waits for
/// a socket, and nothing in the code or the docs would have said 5 s.
const NATS_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a leg gets, and the record of the tolerance that was tried here and
/// **withdrawn**.
///
/// A fixture tolerance (60 s per leg under `cfg(test)`) was implemented and
/// measured against the hostile rig of `docs/testing/flaky-tauri.md` §13.6 —
/// `taskpolicy -b` + 40 burners, two concurrent rigs, `-j 32`, load 60–75, the
/// window in which the pre-fix binary reproduced the failure in 4 of 4 runs. It
/// did not turn those runs green: it turned them into **900 s timeouts of the
/// whole filter** (both instances hit the harness cap; the pre-fix binary had
/// finished the same filter in 370–415 s with 13 failures). Starvation scales
/// every wait, including the tolerance, so a larger wall clock buys nothing at
/// the load where the failure is reachable — and at the loads this host can
/// otherwise produce the failure does not fire at all (0 budget-class failures
/// in 26 natural runs, §13.3). The tolerance's cost (a 12× longer worst case for
/// every future run) is real and its benefit is unmeasurable here, so the shape
/// is withdrawn and the finding is recorded instead: what matters in production
/// is not the budget but *what the client concludes* from a timeout (see the
/// next step in §13.6 — a timeout currently drops the connection, which is how a
/// slow host becomes `peer_not_connected`).
fn request_budget() -> Duration {
    PHONE_REQUEST_TIMEOUT
}

/// The socket-connect leg, named so the option below cannot be inherited by
/// accident. See [`request_budget`] for the tolerance that was tried here.
fn connect_budget() -> Duration {
    NATS_CONNECT_TIMEOUT
}

/// How many times `async-nats` may retry a dropped socket before it gives up.
///
/// The runtime owns reconnection: every retry there is a fresh handshake, with a
/// grant that `connect_attempt` renews first. The library's own loop has no such
/// hook — it re-presents the JWT the socket was opened with, which the relay has
/// already refused if that grant expired, and it never ends. The connection then
/// looks alive forever: no reconnect ever runs, and `connected: true` keeps being
/// reported for a socket that cannot authenticate (the desktop log's
/// `authorization violation` every two seconds).
///
/// One attempt keeps an ordinary blip transparent — the library retries at once
/// (its first backoff is 0 ms) — and if the socket really is gone the connection
/// closes and the bounded supervisor takes over. `async-nats` maps
/// `max_reconnects(0)` to "no limit", so one is the least that still gives up;
/// the test suite's `nats_connect_once` uses the same encoding.
const NATS_RECONNECT_BUDGET: usize = 1;

/// Handshake legs are small and the host expires pending candidates after 30 s;
/// 10 s matches the phone and leaves room for one retry inside that window.
const HANDSHAKE_TIMEOUT: Duration = PHONE_REQUEST_TIMEOUT;

/// What one handshake request gets.
fn handshake_budget() -> Duration {
    HANDSHAKE_TIMEOUT
}

/// Ordinary commands default, matching the phone's request timeout.
pub const COMMAND_TIMEOUT: Duration = PHONE_REQUEST_TIMEOUT;

/// How many attempts a retryable command gets in total, and the base of the
/// `250·2ⁿ ms` backoff between them, matching `client.ts::requestRetry`.
/// TS may add a transport-probe wait; Rust's outer runtime owns recovery.
pub const COMMAND_ATTEMPTS: usize = 3;
pub const RETRY_BACKOFF_BASE: Duration = Duration::from_millis(250);

/// Whether a command may be re-sent after a transport failure.
///
/// Reads are safe to retry. Conversation writes additionally require a stable
/// command id and negotiated durable receipts (see `retryable_for_features`).
/// Other writes remain single-attempt.
pub fn retryable_command(payload: &Value) -> bool {
    let Some(kind) = payload.get("type").and_then(Value::as_str) else {
        return false;
    };
    kind.starts_with("get_")
        || kind.starts_with("list_")
        || kind == "upload_complete"
        || matches!(kind, "prompt" | "continue_run")
}

pub(crate) fn retryable_for_features(payload: &Value, features: &[String]) -> bool {
    if !retryable_command(payload) {
        return false;
    }
    if matches!(
        payload.get("type").and_then(Value::as_str),
        Some("prompt" | "continue_run")
    ) {
        return payload
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
            && features
                .iter()
                .any(|feature| feature == "prompt_receipt_v1");
    }
    true
}

/// One immutable command body for the initial attempt and every retry.
pub(crate) fn command_plaintext(mut payload: Value, bridge: &str) -> Result<Vec<u8>, crate::Error> {
    if let Some(object) = payload.as_object_mut() {
        object
            .entry("bridgeInstanceId")
            .or_insert_with(|| json!(bridge));
    }
    serde_json::to_vec(&payload)
        .map_err(|error| crate::Error::Message(format!("encode command: {error}")))
}

/// The production loop accepts the serving exchange; unit drivers supply only
/// transport outcomes and advance Tokio's virtual clock, not a second policy.
pub(crate) async fn retry_request<'a, T, F, Fut>(
    plaintext: &'a [u8],
    attempts: usize,
    mut exchange: F,
) -> Result<T, crate::Error>
where
    F: FnMut(&'a [u8]) -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::Error>>,
{
    let mut attempt = 1;
    loop {
        match exchange(plaintext).await {
            Err(error) if attempt < attempts && transient(&error) => {
                // Between attempt n and n+1: 250 ms, then 500 ms.
                tokio::time::sleep(RETRY_BACKOFF_BASE * 2u32.pow(attempt as u32 - 1)).await;
                attempt += 1;
            }
            outcome => return outcome,
        }
    }
}

/// A failure worth another attempt: the transport (a reply that never arrived,
/// a socket that went away). A business failure is the host's *answer* — it
/// would compute the same one again — so it is returned at once, exactly as
/// `remote_peer::runtime` reads it.
fn transient(error: &crate::Error) -> bool {
    matches!(error, crate::Error::RemoteTransport(_))
}

/// Capability a client declares on `secure_ready` to be told a catalog row's new
/// recency instead of being sent the host's whole session list again.
///
/// A streaming run rewrites its conversation's activity timestamp about once a
/// second, and that value is the list's ordering key — so without this the host
/// re-sends every conversation it owns, once per second, for the length of the
/// run (measured at 101.7–178.8 MB per run on the six heaviest completed runs).
/// The client appends the packet to its catalogue in
/// `desktop/src/components/layout/hooks/useRemotePeers.ts`.
pub const CATALOG_ACTIVITY_FEATURE: &str = "catalog_activity_v1";

/// Capability a client declares on `secure_ready` to narrow its event lane from
/// "every conversation on the host" to the one the user is looking at.
///
/// Unlike the two content flags this one is *negotiated*: the host answers with
/// the capability list it supports, and the narrowing is only applied when
/// `selective_events_v1` comes back in it. A host that does not know the flag
/// keeps sending the wide lane, and a client that narrowed anyway would show a
/// blank conversation.
pub const SELECTIVE_EVENTS_FEATURE: &str = "selective_events_v1";

/// Capabilities this client declares on `secure_ready`, in the order they are
/// advertised. Kept as a named constant so the declaration is testable without
/// a live host: without one, a future edit could quietly drop a capability the
/// decode path depends on.
///
/// `event_coalescing_v1` needs no consumer change (see `super::codec` and the
/// handshake comment); `reply_gzip_v1` is only safe because `super::codec`
/// inflates bounded replies; `selective_events_v1` is only safe because the
/// narrow lane is paired with the `state.events` subscription
/// ([`PeerSession::state_subject`]), which is what keeps the host's cross-session
/// notifications arriving; `lean_events_v1` is only safe because what it strips
/// has somewhere else to come from, and because the read it unlocks is done:
///
/// * `sessionId` — kept in the trimmed body itself (`remote_host::lean`), because
///   the `state.events` mirror's subject names no conversation; the subject is
///   also recovered into the payload (`runtime::event_session_id`), which is what
///   the phone does and what covers a host that trims the key.
/// * `eventId` / `timestamp` — the timeline's live entry falls back to the record
///   inside `data` and then to `{runId}:{idx}` (`useRemoteTimeline`).
/// * a shell call's arguments — the host serves `get_tool_call_args` **only** to a
///   client that declared this, and lean is what takes them off the page. The two
///   are one mechanism: declaring one without consuming the other would leave
///   every shell row showing its identity and no command.
/// * `catalog_activity_v1` — the host stops re-sending a whole session list for
///   activity, which this client is only safe to ask for while it *applies* the
///   replacement packet (`useRemotePeers::handleCatalog`, domain
///   `sessions_activity`). The packet is then the only writer of a remote row's
///   ordering timestamp on this side: a live event moves no row, because a
///   client clock in that value is a row the next packet moves back.
pub const DECLARED_FEATURES: [&str; 8] = [
    "event_coalescing_v1",
    future_remote_protocol::SEGMENTED_CONTENT_FEATURE,
    "reply_gzip_v1",
    SELECTIVE_EVENTS_FEATURE,
    future_remote_protocol::LEAN_EVENTS_FEATURE,
    CATALOG_ACTIVITY_FEATURE,
    // The promise that goes with this declaration: beats while this client is
    // here, a goodbye when it leaves. A host that knows it presumes this client
    // gone when the beats lapse, which is what keeps the Remote page from saying
    // "in control" about a machine whose client was closed.
    future_remote_protocol::CLIENT_PRESENCE_FEATURE,
    // A paged read's chunk bytes as raw bytes behind a JSON header instead of
    // base64 inside it. This is the one entry that is *conditional* — see
    // [`declared_features`]: the encoding it selects is the host's answer to it,
    // so it is only declared once the host has said it implements the frame.
    future_remote_protocol::READ_BINARY_FEATURE,
];

/// What this connection actually declares on `secure_ready`.
///
/// Everything in [`DECLARED_FEATURES`] except the negotiated ones, whose
/// declaration is itself the offer a host has to have answered first.
///
/// `read_binary_v1` is the only one today. It changes the *encoding* of a reply,
/// which is not something either end can fall back from at the last moment: a
/// declaration sent to a host that advertises the flag but chose not to honour
/// it (a build that disabled it, a future version serving a different carrier)
/// would put the host in the position of answering in a shape only this client's
/// node expects — and, symmetrically, of depending on this client's decoder
/// supporting a frame it never offered. Reading the confirmation first is what
/// makes each side's behaviour a function of the other's stated capability
/// rather than of a guess.
///
/// `selective_events_v1` is *not* filtered here: it is already declared
/// unconditionally and its effect is gated on the confirmation at the point of
/// use ([`PeerSession::selective_events`]), which is the same guarantee one step
/// later. There is no such point for the frame — the reply is decoded before any
/// caller can weigh it — so the gate has to be here.
fn declared_features(confirmed: &[String]) -> Vec<&'static str> {
    let binary_confirmed = confirmed
        .iter()
        .any(|feature| feature == future_remote_protocol::READ_BINARY_FEATURE);
    DECLARED_FEATURES
        .iter()
        .copied()
        .filter(|feature| {
            binary_confirmed || *feature != future_remote_protocol::READ_BINARY_FEATURE
        })
        .collect()
}

/// How often a connected client tells its host it is still there.
///
/// The host's lease is four beats (`CLIENT_PRESENCE_LEASE` in
/// `future-remote-server`), so one lost beat is not a departure while a client
/// that is really gone drops out of the page within a poll of a minute. The
/// mobile client sends at the same interval (`packages/remote-client/src/client.ts`).
pub const CLIENT_PRESENCE_BEAT: Duration = Duration::from_secs(15);
/// How long one beat may wait for the socket before it is given up.
const PRESENCE_BEAT_BUDGET: Duration = Duration::from_secs(5);
/// A goodbye gets far less: the caller is closing the connection under it.
const PRESENCE_GOODBYE_BUDGET: Duration = Duration::from_millis(750);
/// How long the socket may take to drain the goodbye before it is dropped.
pub(crate) const PRESENCE_CLOSE_BUDGET: Duration = Duration::from_millis(750);

/// The session token a narrow `evt.` subscription may be built from is guarded
/// against interpolation into a subject elsewhere; see [`SubscriptionToken`].
///
/// The id is interpolated into a NATS subject, so a token that is not a plain
/// identifier would not narrow anything: `*` or `>` would *widen* the
/// subscription back to the whole host, and a `.` would address a different
/// subject than the host publishes on. The phone's own predicate
/// (`mobile/src/remote/client.ts::setVisibleSession`) is the same charset, and
/// a rejected id means "no conversation is being viewed" — the wide lane —
/// rather than a subscription nobody can explain.
pub fn narrowable_session(session_id: &str) -> Option<&str> {
    (!session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(session_id)
}

/// Where the host publishes its cross-conversation notifications, alongside the
/// conversation's own `evt.` subject (`remote::publisher::is_catalog_event`).
pub fn state_subject(pair_id: &str) -> String {
    format!("p.{pair_id}.state.>")
}

/// The events subscription `visible` calls for.
///
/// A free function rather than a method so the decision can be tested without a
/// socket: it is the whole of "which lane", and every path into it — the initial
/// `SUB` and the move when the user opens another conversation — goes through
/// this one construction, so a reader of `state.events` and the subscription it
/// belongs to can never disagree about which lane is installed.
pub fn events_lane(
    pair_id: &str,
    narrow_confirmed: bool,
    visible: Option<&str>,
) -> EventsSubscription {
    match visible
        .filter(|_| narrow_confirmed)
        .and_then(narrowable_session)
    {
        Some(session) => EventsSubscription {
            subject: format!("p.{pair_id}.evt.{session}"),
            narrow: true,
        },
        None => EventsSubscription {
            subject: format!("p.{pair_id}.evt.>"),
            narrow: false,
        },
    }
}

/// The read lanes one connection drains, taken together at connect time.
///
/// One struct rather than a tuple: `spawn_event_stream` takes all four and the
/// call site has to keep them straight (an `events`/`presence` swap would compile
/// and deliver presence ticks to the transcript).
pub struct Subscriptions {
    pub events: async_nats::Subscriber,
    pub presence: async_nats::Subscriber,
    pub state: async_nats::Subscriber,
    pub transfers: async_nats::Subscriber,
}

/// Everything needed to point the events lane at a conversation and to move it
/// when the user opens another one: the pair it belongs to, the socket, and
/// whether the host confirmed the capability.
///
/// The session is *not* part of it. Holding it here would make the lane a
/// snapshot of one moment, and the whole point of the type is that the lane
/// moves while the connection stays up.
pub struct EventLane {
    pair_id: String,
    client: async_nats::Client,
    narrow_confirmed: bool,
}

/// One installed events subscription: where it listens, and whether that is the
/// narrowed lane.
///
/// Both facts travel together because they are one decision. Whether the
/// reader of `state.events` double-reads a host-wide event depends on which lane
/// is installed, and a caller that recomputed the predicate itself could answer
/// differently from the subscription it just opened.
pub struct EventsSubscription {
    pub subject: String,
    pub narrow: bool,
}

impl EventLane {
    /// The lane `visible` calls for: the narrowed one when the host confirmed
    /// the capability and the session is a plain token, else the wide lane.
    pub fn for_visible(&self, visible: Option<&str>) -> EventsSubscription {
        events_lane(&self.pair_id, self.narrow_confirmed, visible)
    }

    /// Open (or re-open) the events lane for `visible`.
    ///
    /// A fresh `SUB` rather than a filter change: NATS Core has no "change the
    /// subject back", and the previous subscriber is dropped when it is
    /// replaced, which is also what stops the old conversation's pushes from
    /// arriving after the user has moved on.
    pub async fn subscribe(
        &self,
        visible: Option<&str>,
    ) -> Result<async_nats::Subscriber, crate::Error> {
        subscribe(&self.client, self.for_visible(visible).subject).await
    }
}

pub struct PeerSession {
    pair_id: String,
    client: async_nats::Client,
    /// Per-connection traffic keys. Never persisted, never reused across
    /// connections: a new socket always starts from a new handshake.
    ///
    /// Behind a `std` mutex rather than an async one because the guard is never
    /// held across an `await`: sealing and opening are pure CPU, and holding a
    /// lock while awaiting a reply would serialise the event subscription loop
    /// behind a slow command.
    channel: std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    /// The host's bridge instance, from the handshake confirmation. Sent with
    /// every command so a host that restarted mid-conversation can be told
    /// apart from the one the client handshook with.
    bridge_instance_id: String,
    features: Vec<String>,
    presence: Value,
    control: std::sync::Arc<std::sync::Mutex<Control>>,
}

impl PeerSession {
    pub fn pair_id(&self) -> &str {
        &self.pair_id
    }
    /// Whether this device may drive the host, and who holds it if not.
    ///
    /// `control.state` is `None` on a host that predates the field, which reads
    /// as "assume ours" — the same assumption every client made before the host
    /// had anything to say about it.
    pub fn control(&self) -> Control {
        self.control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The live handle the event stream updates when the host's control moves.
    pub(crate) fn control_for_stream(&self) -> std::sync::Arc<std::sync::Mutex<Control>> {
        self.control.clone()
    }
    pub fn bridge_instance_id(&self) -> &str {
        &self.bridge_instance_id
    }

    pub fn features(&self) -> &[String] {
        &self.features
    }

    /// Whether the host confirmed the narrowed event lane. Read from the
    /// confirmation rather than from what this client asked for: a host that
    /// does not implement the flag would never send the cross-session events the
    /// narrow lane has to give up, and a client that narrowed anyway would go
    /// quiet without saying so.
    pub fn selective_events(&self) -> bool {
        self.features
            .iter()
            .any(|feature| feature == SELECTIVE_EVENTS_FEATURE)
    }

    /// Where this connection reads session events.
    ///
    /// Wide (`p.{pair}.evt.>`) unless every one of three things holds: the host
    /// confirmed the capability, the user is looking at a conversation, and that
    /// conversation's id is a plain token ([`narrowable_session`]). Otherwise
    /// the subscription is the one this client has always used, so the fallback
    /// is the *existing* behaviour rather than a second lane to reason about.
    pub fn event_lane(&self) -> EventLane {
        EventLane {
            pair_id: self.pair_id.clone(),
            client: self.client.clone(),
            narrow_confirmed: self.selective_events(),
        }
    }

    /// Where the host mirrors the handful of low-rate events that concern a
    /// host as a whole rather than one conversation: `approval_request`,
    /// `approval_decision`, `agent_start`/`agent_end` and the settings changes
    /// (`remote::publisher::is_catalog_event`).
    ///
    /// Subscribed to for exactly as long as the narrow lane is in use. The host
    /// publishes each of those events twice — once on its conversation's `evt.`
    /// subject and once here — so a client on the wide lane already has them and
    /// reading this too would deliver every approval card twice. On the narrow
    /// lane this is the *only* place a non-visible conversation's approval or
    /// activity can still arrive, which is why narrowing without it silently
    /// stops the peer list and the approval card from updating.
    pub fn state_subject(&self) -> String {
        state_subject(&self.pair_id)
    }

    /// Whether the host said, at handshake time, that its agent was reachable.
    /// A reachable bridge with an unavailable agent is `LC003` in the support
    /// table: the link is up and every command fails.
    pub fn agent_available(&self) -> bool {
        self.control()
            .agent_available
            .unwrap_or_else(|| agent_available(&self.presence))
    }

    /// The traffic-key mutex, shared with the event stream task. The task takes
    /// this same lock, so a stream and a concurrent command both work without
    /// holding it across an await.
    pub fn channel_for_stream(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>> {
        self.channel.clone()
    }

    /// The underlying connection, so a test can drive a handshake on a socket it
    /// has closed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn client_for_test(&self) -> &async_nats::Client {
        &self.client
    }

    /// A session wired to a broker and a channel the *test* owns, with no
    /// handshake in front of it.
    ///
    /// The retry is about re-sending, so a test of it wants to be the host: to
    /// see each request and to choose whether to answer. `connect` cannot offer
    /// that — its host always replies — and a broker that is up cannot lose one
    /// reply on demand.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        pair_id: &str,
        client: async_nats::Client,
        channel: std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
        bridge_instance_id: &str,
    ) -> Self {
        Self {
            pair_id: pair_id.to_string(),
            client,
            channel,
            bridge_instance_id: bridge_instance_id.to_string(),
            features: Vec::new(),
            presence: Value::Null,
            control: std::sync::Arc::new(std::sync::Mutex::new(Control::default())),
        }
    }

    /// Drain the connection: every subscription this session holds ends.
    ///
    /// `drain` rather than a bare drop, because draining is what actually
    /// *ends the streams* — the state a process shutdown leaves behind, and the
    /// only way a stream task can observe that its socket is gone. Killing the
    /// broker does not do it: the client reconnects and keeps the subscriptions
    /// open.
    /// Whether the transport itself is still up, as the NATS client sees it.
    ///
    /// This is the distinction a *timed-out request* cannot make on its own: a
    /// slow host and a dead one look identical from the caller's seat, and the
    /// client is the only component that knows whether the socket beneath the
    /// request is still there. `Connected` means the client has a live
    /// connection to the broker (its own PING/PONG bookkeeping is what moves it
    /// off `Connected`), so a request that timed out over a `Connected` client is
    /// a **slow peer**, not a lost connection.
    ///
    /// Used by `runtime::on_transport_failure`, which is what decides whether a
    /// failed command also means "this host is gone".
    pub fn is_connected(&self) -> bool {
        matches!(
            self.client.connection_state(),
            async_nats::connection::State::Connected
        )
    }

    pub async fn close_socket(&self) -> Result<(), crate::Error> {
        self.client
            .drain()
            .await
            .map_err(|error| crate::Error::RemoteTransport(error.to_string()))
    }

    /// Subscribe to the host's event, presence, host-wide and file-transfer
    /// streams, on the lane `visible` calls for.
    ///
    /// `p.{pair}.evt.{session}` (or `.evt.>` when nothing is being viewed)
    /// carries session events, `p.{pair}.presence` the host's liveness,
    /// `p.{pair}.state.>` the host-wide mirror and `p.{pair}.xfer.down.>`
    /// the file bytes a `pull` produces. All of them are decrypted with the same
    /// channel as commands, so a push is authenticated exactly as strongly as a
    /// reply.
    ///
    /// The transfer subscription is taken at connect time rather than around a
    /// download because of the order the host uses: it publishes the chunk
    /// *before* it acknowledges the pull (so the bytes are observably on the
    /// wire first). A subscription opened after the pull request would therefore
    /// race the very publish it is waiting for, and NATS Core does not replay.
    pub async fn subscribe(&self, visible: Option<&str>) -> Result<Subscriptions, crate::Error> {
        let events =
            subscribe(&self.client, self.event_lane().for_visible(visible).subject).await?;
        let presence = subscribe(&self.client, self.presence_subject()).await?;
        let state = subscribe(&self.client, self.state_subject()).await?;
        // Chunks only, by shape: `xfer.down.*.chunk.*` is exactly the subject the
        // host publishes a pulled chunk on. The wildcard is what makes the
        // receive path's parse total — a delivery that is not a chunk cannot
        // arrive, so there is no "what else could this be" to answer.
        let transfers = subscribe(
            &self.client,
            format!("p.{}.xfer.down.*.chunk.*", self.pair_id),
        )
        .await?;
        // SUB enqueueing is not broker readiness, and this flush does not make it
        // so: `Client::flush()` writes the buffered frames and awaits no
        // acknowledgement (async-nats 0.50 has no ping/pong round trip on it). It
        // buys exactly one thing — the `SUB` frames are on the socket, not in a
        // buffer the client might still be batching — so the window in which a
        // publish from another connection can beat our registration is as small
        // as this process can make it. What it does not buy: a host publishing
        // in that window still loses the event (NATS Core has no replay), which
        // is why a redelivery-shaped gap is healed by `get_events_since` and why
        // a test that publishes immediately after connect must wait for the
        // broker's own subscription table (`FakeNats::wait_for_sub`), not for
        // this call to return.
        tokio::time::timeout(request_budget(), self.client.flush())
            .await
            .map_err(|_| {
                crate::Error::RemoteTransport("Remote subscription flush timed out".into())
            })?
            .map_err(|error| crate::Error::RemoteTransport(error.to_string()))?;
        Ok(Subscriptions {
            events,
            presence,
            state,
            transfers,
        })
    }

    pub fn presence_subject(&self) -> String {
        format!("p.{}.presence", self.pair_id)
    }

    /// Where a chunk pull is requested: the host's upload lane, since the client
    /// is pulling *from* it.
    pub fn transfer_pull_subject(&self, transfer_id: &str, index: u64) -> String {
        format!("p.{}.xfer.up.{transfer_id}.pull.{index}", self.pair_id)
    }

    /// Where one upload chunk is written. The host routes the trailing tokens
    /// (`chunk`/`pull`) itself, so the two directions share this prefix.
    pub fn transfer_chunk_subject(&self, transfer_id: &str, index: u64) -> String {
        format!("p.{}.xfer.up.{transfer_id}.chunk.{index}", self.pair_id)
    }

    /// A command subject for `session_key`: the host routes on the trailing
    /// token, and `list` / `new` are its two non-session lanes.
    pub fn command_subject(&self, session_key: &str) -> String {
        command_subject(&self.pair_id, session_key)
    }

    /// Send one sealed command without waiting for a reply.
    ///
    /// Presence beats and the departure notice use this: they are proof and
    /// courtesy rather than requests, and a client that had to wait for the host
    /// to answer its goodbye would be closed before it could say it. Bounded by
    /// the caller's own budget, never retried — the host's lease is what covers
    /// a beat that never arrived.
    pub async fn notify(
        &self,
        subject: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<(), crate::Error> {
        let plaintext = command_plaintext(payload, &self.bridge_instance_id)?;
        let wire = self
            .channel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .seal(subject, &plaintext)
            .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
        // Publish *and flush*: a publish only queues the frame, and the socket is
        // about to be closed under this one (`disconnect`, app exit). Waiting for
        // the client's own periodic flush would lose the goodbye exactly when it
        // matters — the last message a client ever sends is the one nobody can
        // retry.
        tokio::time::timeout(timeout, async {
            self.client
                .publish(subject.to_string(), wire.into())
                .await
                .map_err(|error| {
                    crate::Error::RemoteTransport(format!("Remote publish failed: {error}"))
                })?;
            self.client.flush().await.map_err(|error| {
                crate::Error::RemoteTransport(format!("Remote flush failed: {error}"))
            })
        })
        .await
        .map_err(|_| crate::Error::RemoteTransport("Remote publish timed out".into()))?
    }

    /// "I am still here."
    ///
    /// Sent on a timer for the life of this connection (`runtime::spawn_presence_beats`),
    /// because an idle client says nothing else: without it, a host that knows
    /// [`future_remote_protocol::CLIENT_PRESENCE_FEATURE`] would have to read
    /// silence as absence, and every quiet minute would look like a departure.
    pub async fn beat_presence(&self) -> Result<(), crate::Error> {
        let subject = self.command_subject("client_presence");
        self.notify(
            &subject,
            json!({ "type": "client_presence" }),
            PRESENCE_BEAT_BUDGET,
        )
        .await
    }

    /// "I am leaving."
    ///
    /// `reason` is the difference between the two halves of leaving: `"disconnected"`
    /// is the user's own stop and gives up the machine this device was driving,
    /// while `"background"` only stops the host drawing it as present (a suspended
    /// app comes back, and takes its machine with it).
    pub async fn leave(&self, reason: &str) -> Result<(), crate::Error> {
        let subject = self.command_subject("client_disconnect");
        self.notify(
            &subject,
            json!({ "type": "client_disconnect", "reason": reason }),
            PRESENCE_GOODBYE_BUDGET,
        )
        .await
    }

    /// Send one encrypted command and decode its reply.
    ///
    /// `bridge_instance_id` is attached when the caller is addressing an
    /// already-established conversation: a host that has since restarted
    /// refuses it instead of applying it to a session it never saw.
    ///
    /// A command on [`retryable_command`]'s whitelist is re-sent after a
    /// transport failure (see [`Self::exchange_retrying`]); every other command
    /// makes exactly one attempt.
    ///
    /// `&self` rather than `&mut self`: the runtime keeps a host's session
    /// behind an `Arc` so a caller can await a round trip *without* the
    /// process-wide runtime lock. Nothing here mutates the session — the traffic
    /// keys have their own mutex and the NATS client is shared by design — so
    /// the exclusive borrow was only ever a signature, and it was what forced
    /// the lock to be held across a network wait (and therefore what made one
    /// host's slow command stall the peer list of every other host).
    pub async fn request(
        &self,
        subject: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<Reply, crate::Error> {
        // Read before the envelope fields are added: the whitelist is a property
        // of the command, not of the lane it is sent on.
        let retryable = retryable_for_features(&payload, &self.features);
        let plaintext = command_plaintext(payload, &self.bridge_instance_id)?;
        if retryable {
            self.exchange_retrying(subject, &plaintext, timeout, COMMAND_ATTEMPTS)
                .await
        } else {
            self.exchange(subject, &plaintext, timeout).await
        }
    }

    /// Send arbitrary bytes and decode the reply.
    ///
    /// Split out because not every request body is a command document: an
    /// upload chunk *is* its payload, and wrapping it in JSON would both inflate
    /// it and lose the exact bytes the host hashes.
    ///
    /// Never retried here: a body with no `type` is not on the whitelist, and
    /// the chunk lane has its own retry (`transfer::retrying`).
    pub async fn request_bytes(
        &self,
        subject: &str,
        plaintext: &[u8],
        timeout: Duration,
    ) -> Result<Reply, crate::Error> {
        self.exchange(subject, plaintext, timeout).await
    }

    /// Re-send the same sealed plaintext while the failure is a transport one.
    ///
    /// The plaintext — not the `Value` — is what is re-sent, so the command `id`
    /// and every other byte of the request are identical between attempts. That
    /// is what makes the retry idempotent: the host keys its reply cache on the
    /// `id` and refuses a reused id whose body differs (`command_id_conflict`),
    /// so its second look-up replays the first answer instead of executing the
    /// command again.
    ///
    /// The *last* error is the one returned, so a caller sees the failure that
    /// exhausted the budget rather than the first one.
    async fn exchange_retrying(
        &self,
        subject: &str,
        plaintext: &[u8],
        timeout: Duration,
        attempts: usize,
    ) -> Result<Reply, crate::Error> {
        retry_request(plaintext, attempts, |body| {
            self.exchange(subject, body, timeout)
        })
        .await
    }

    /// Seal a request, await its reply, and unwrap the host's envelope.
    ///
    /// The reply is decoded by *shape*, not by what this client asked for: a
    /// body that starts with the binary frame's magic is a page carrying its
    /// bytes outside its JSON, and anything else is a JSON document. That is
    /// what lets one client read both carriers — a host that never learned the
    /// negotiation keeps sending base64 pages, and this reader is the same code
    /// path for both.
    async fn exchange(
        &self,
        subject: &str,
        plaintext: &[u8],
        timeout: Duration,
    ) -> Result<Reply, crate::Error> {
        let wire = self
            .channel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .seal(subject, plaintext)
            .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
        let response = tokio::time::timeout(
            timeout,
            self.client
                .request(subject.to_string(), wire.clone().into()),
        )
        .await
        .map_err(|_| crate::Error::RemoteTransport("Remote request timed out".into()))?
        .map_err(|error| {
            crate::Error::RemoteTransport(format!("Remote request failed: {error}"))
        })?;
        let context = future_remote_crypto::reply_context(subject, &wire)
            .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
        let opened = self
            .channel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .open(&context, &response.payload)
            .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
        let plain = super::codec::plain_json_bytes(&opened)
            .map_err(|error| crate::Error::Message(format!("decode reply: {error}")))?;
        let frame = super::codec::split_binary_frame(&plain)
            .map_err(|error| crate::Error::Message(format!("decode reply: {error}")))?;
        let reply = match frame {
            Some(frame) => {
                let header: Value = serde_json::from_slice(frame.header)
                    .map_err(|error| crate::Error::Message(format!("decode reply: {error}")))?;
                check_reply_success(&header)?;
                Reply::Page {
                    data: header.get("data").cloned().unwrap_or(Value::Null),
                    body: frame.body.to_vec(),
                }
            }
            None => {
                let document: Value = serde_json::from_slice(&plain)
                    .map_err(|error| crate::Error::Message(format!("decode reply: {error}")))?;
                check_reply_success(&document)?;
                Reply::Data(document.get("data").cloned().unwrap_or(Value::Null))
            }
        };
        Ok(reply)
    }
}

/// The host's answer, as far as the body tells us.
///
/// Both carriers wrap the same envelope, so the refusal test lives once: a
/// business failure is the host's *answer* and is reported as the message the
/// caller can act on, never as a decode fault.
fn check_reply_success(envelope: &Value) -> Result<(), crate::Error> {
    if envelope.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let detail = envelope
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("command_failed");
    Err(crate::Error::Message(detail.to_string()))
}

/// One decoded reply.
///
/// Two shapes because the wire has two, and the difference is not a detail a
/// caller can ignore: a page's bytes exist only in [`Reply::Page`], while every
/// other reply's `data` is the whole answer. The envelope's `data` is available
/// from either, so code that only inspects *what* the host said (the paging
/// address, a catalogue snapshot) does not have to match on the carrier.
pub enum Reply {
    /// The `data` of an ordinary JSON reply.
    Data(Value),
    /// A negotiated binary page: the same `data` a JSON reply would carry
    /// (`{"readChunk": {…}}`) plus the bytes that travelled outside it.
    Page { data: Value, body: Vec<u8> },
}

impl Reply {
    /// The envelope's `data`, whichever carrier it arrived on.
    pub fn data(&self) -> &Value {
        match self {
            Reply::Data(data) => data,
            Reply::Page { data, .. } => data,
        }
    }

    /// The envelope's `data`, dropping any carried bytes.
    pub fn into_data(self) -> Value {
        match self {
            Reply::Data(data) => data,
            Reply::Page { data, .. } => data,
        }
    }

    /// The page's bytes, when this reply carried them outside its JSON.
    ///
    /// `None` is not an error: it is the base64 carrier, where the bytes are
    /// inside `data` and a reader decodes them from there.
    pub fn into_page(self) -> (Value, Option<Vec<u8>>) {
        match self {
            Reply::Data(data) => (data, None),
            Reply::Page { data, body } => (data, Some(body)),
        }
    }
}

/// A page's body is up to a whole chunk, so it is described by its length: a
/// `Debug` that printed it would put 192 KiB of ciphertext-adjacent bytes into
/// every failure message and every test's `expect_err`.
impl std::fmt::Debug for Reply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Reply");
        match self {
            Reply::Data(data) => debug.field("data", data).finish(),
            Reply::Page { data, body } => debug
                .field("data", data)
                .field("bodyBytes", &body.len())
                .finish(),
        }
    }
}

/// The one place the trailing routing token is built. An empty session key is
/// the "new conversation" lane the host serves, not a subject ending in a dot
/// (which the host's `cmd.>` subscription would still receive, but whose dedup
/// and reply routing differ).
pub fn command_subject(pair_id: &str, session_key: &str) -> String {
    let key = if session_key.is_empty() {
        "new"
    } else {
        session_key
    };
    format!("p.{pair_id}.cmd.{key}")
}

/// What the host's handshake confirmation has to say for the connection to be
/// usable.
///
/// A pure validator, deliberately: every rejection here is a *protocol* fault a
/// real host should never produce, so the only way to exercise them is to feed
/// in the malformed confirmation a buggy or hostile peer would send — which
/// cannot be arranged against a real host by construction.
#[derive(Debug, PartialEq, Eq)]
pub struct Confirmed {
    pub bridge_instance_id: String,
    pub features: Vec<String>,
    pub presence: Value,
}

impl Confirmed {
    pub fn from_confirmation(confirmation: &Value, pair_id: &str) -> Result<Self, crate::Error> {
        let invalid = || crate::Error::Message("remote_secure_channel_invalid".into());
        let bridge_instance_id = confirmation
            .get("bridgeInstanceId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?
            .to_string();
        // The confirmation is the host's statement about *this* pairing. A reply
        // that confirms a different pair, or that does not claim v2, is a
        // protocol mismatch rather than a degraded session — accepting it would
        // mean trusting an identity the handshake never proved.
        if confirmation.get("confirmed").and_then(Value::as_bool) != Some(true)
            || confirmation.get("pairId").and_then(Value::as_str) != Some(pair_id)
        {
            return Err(invalid());
        }
        let features: Vec<String> = confirmation
            .get("features")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if !features.iter().any(|feature| feature == "e2ee_v2") {
            return Err(invalid());
        }
        Ok(Self {
            bridge_instance_id,
            features,
            presence: confirmation.get("presence").cloned().unwrap_or(Value::Null),
        })
    }
}

/// The host's own statement about its agent, read from the handshake presence.
///
/// Absent or non-boolean reads as `false`, not `true`: the value gates a UI that
/// says "everything is fine", so an older host that does not report the field
/// must not be assumed healthy — the user would find out from a failed command
/// instead.
pub fn agent_available(presence: &Value) -> bool {
    presence
        .get("agentAvailable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether this device may drive the host, as the host last stated it.
///
/// The host states it in two places, and this type is what they both land in:
/// its presence packet (`control`, plus `holder` when somebody else has it) and
/// a `p.{pair}.state.control` packet published on a transition, which names the
/// state and not the holder. Everything else reads this copy — the machine list
/// shows it, and a command refused for `superseded` is explained by it.
///
/// `state: None` is a host that predates the field. It means "assume ours",
/// which is what every client did before the host could say otherwise; reading
/// silence as standby would make every older host look unusable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Control {
    pub agent_available: Option<bool>,
    pub state: Option<ControlState>,
    pub holder: Option<Holder>,
}

impl Control {
    /// The state a handshake's presence declared.
    pub(crate) fn from_presence(presence: &Value) -> Self {
        let mut control = Self::default();
        control.apply_presence(presence);
        control
    }

    /// Take a full statement of control: the handshake presence, or a tick.
    ///
    /// A presence payload without `control` is an older host and changes
    /// nothing — its silence is not a state.
    pub(crate) fn apply_presence(&mut self, presence: &Value) {
        if let Some(available) = presence.get("agentAvailable").and_then(Value::as_bool) {
            self.agent_available = Some(available);
        }
        let Some(state) = ControlState::parse(presence.get("control")) else {
            return;
        };
        self.state = Some(state);
        self.holder = match state {
            ControlState::Active => None,
            ControlState::Standby => presence.get("holder").and_then(Holder::read),
        };
    }

    /// Take a `state.control` packet, which names the state and not the holder.
    ///
    /// The holder is cleared rather than kept: on a move to standby, what is
    /// held here still names the device that was just displaced. The next
    /// presence (≤ one tick away) names the new one, and until then the client
    /// says "another device" instead of the wrong name.
    pub(crate) fn apply_control(&mut self, packet: &Value) {
        let Some(state) = ControlState::parse(packet.get("control")) else {
            return;
        };
        self.state = Some(state);
        self.holder = None;
    }
}

/// `active` (this device drives the host) or `standby` (another one does).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlState {
    Active,
    Standby,
}

impl ControlState {
    fn parse(value: Option<&Value>) -> Option<Self> {
        match value?.as_str()? {
            "active" => Some(Self::Active),
            "standby" => Some(Self::Standby),
            // A word this client does not know is not a state to act on: the
            // machine stays as it was, rather than being declared unusable.
            _ => None,
        }
    }
}

/// Who holds the host while this device is in standby.
///
/// The controller's self-declared identity, from its `secure_ready`: it is what
/// lets the window say "taken over by «name»" instead of refusing without saying
/// why. Every field is optional — a controller that never declared anything
/// leaves the pair id, or nothing at all, and the client falls back to neutral
/// wording rather than inventing an identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl Holder {
    fn read(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let field = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let holder = Self {
            pair_id: field("pairId"),
            name: field("name"),
            kind: field("kind"),
        };
        (!holder.is_empty()).then_some(holder)
    }

    fn is_empty(&self) -> bool {
        self.pair_id.is_none() && self.name.is_none() && self.kind.is_none()
    }
}

/// A completed connection, plus what the handshake learned about the pairing.
pub struct Connected {
    pub session: PeerSession,
    /// True when this connection consumed the invitation PSK, i.e. it was the
    /// first successful pairing. The caller must then drop the stored secret:
    /// the host has already dropped its own copy, and keeping ours would leave
    /// a dead-but-real bearer credential on disk.
    pub consumed_invitation: bool,
}

/// Map the library's connect-failure kind onto this crate's two kinds.
///
/// `Authentication`/`AuthorizationViolation` are the credential being refused:
/// `AppError::RemoteAuthorization`, which the runtime treats as permanent (a
/// retry cannot repair it, and retrying would hide the re-pair the user has to
/// do). Every other kind — `TimedOut`, `Io`, `Dns`, `MaxReconnects`, `Tls`,
/// `ServerParse` — is `AppError::RemoteTransport`, the transient class the
/// runtime may retry.
///
/// A free function so both arms are asserted directly
/// (`connect_failures_are_classified_by_their_kind`), instead of needing a
/// fixture that can make a real broker refuse authorization.
fn classify_connect_error(kind: async_nats::ConnectErrorKind, message: &str) -> crate::Error {
    match kind {
        async_nats::ConnectErrorKind::Authentication
        | async_nats::ConnectErrorKind::AuthorizationViolation => {
            crate::Error::RemoteAuthorization(message.to_string())
        }
        _ => crate::Error::RemoteTransport(message.to_string()),
    }
}

/// Connect and authenticate. One attempt, no retry policy (the supervisor owns
/// that), but the two handshake patterns are tried in the only order that is
/// safe: PSK pairing first, then `IK` with the pinned key.
pub async fn connect(
    creds: &PeerCreds,
    environment: &ConnectionOptions,
) -> Result<Connected, crate::Error> {
    let key_pair = std::sync::Arc::new(
        nkeys::KeyPair::from_seed(&creds.nkey_seed).map_err(|error| {
            crate::Error::Message(format!("Invalid stored device NKey: {error}"))
        })?,
    );
    let signer = key_pair.clone();
    let options = async_nats::ConnectOptions::with_jwt(creds.user_jwt.clone(), move |nonce| {
        let signer = signer.clone();
        async move { signer.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .custom_inbox_prefix(format!("p.{}.rep.{}", creds.pair_id, creds.device_id))
    // Both budgets are set explicitly rather than inherited from `async-nats`:
    // inherited, a dependency bump could change how long the app waits for a
    // socket without anything in this crate saying so, and nothing here named
    // the value. `request_budget`/`connect_budget` carry the values and the
    // record of the fixture tolerance that was tried and withdrawn.
    .connection_timeout(connect_budget())
    .request_timeout(Some(request_budget()))
    // A refused credential must end this generation, not be retried by the
    // library forever; see [`NATS_RECONNECT_BUDGET`].
    .max_reconnects(Some(NATS_RECONNECT_BUDGET));
    // Same platform-store read as the bridge's own connection (`remote::trust`):
    // the intermittent failure is in reading the store, not in the handshake.
    let options = match environment.root_bundle.as_ref() {
        Some(bundle) => options.add_root_certificates(bundle.clone()),
        None => options,
    };
    // The pairing's token URL records whether its self-hosted relay declared
    // plaintext transport. Platform/HTTPS pairings continue to require TLS.
    // Test-only plaintext fixtures still validate loopback before connecting.
    let plaintext = creds.token_url.starts_with("http://");
    let options = if environment.insecure_loopback() {
        let endpoint = url::Url::parse(&creds.nats_url)
            .map_err(|_| crate::Error::Message("remote_test_endpoint_invalid".into()))?;
        if !endpoint
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1"))
        {
            return Err(crate::Error::Message(
                "remote_test_endpoint_not_loopback".into(),
            ));
        }
        options
    } else if plaintext {
        options
    } else {
        options.require_tls(true)
    };
    // `async-nats` resolves inside itself and offers no hook to replace that, so
    // when this process has its own resolver the address is resolved here and
    // the connection is pinned to the name it was resolved from; see
    // [`crate::relay_endpoint`].
    let endpoint = future_relay_endpoint::for_connect(
        &creds.nats_url,
        environment.root_bundle.as_deref(),
        plaintext,
    )
    .await
    .map_err(|error| {
        if error.transient {
            crate::Error::RemoteTransport(error.message)
        } else {
            crate::Error::Message(error.message)
        }
    })?;
    let options = match endpoint.tls {
        Some(config) => options.tls_client_config(config),
        None => options,
    };
    let client = options.connect(&endpoint.url).await.map_err(|error| {
        // Two different kinds of failure, which need two different responses from
        // the caller: an authorization failure will never succeed on a retry (the
        // credential is the problem), while everything else on this leg — a
        // socket that timed out, a refused connection, DNS — is the network, and
        // is exactly the case a bounded retry exists for.
        //
        // Classified by the library's own kind rather than by the message text,
        // because "timed out" is a phrase several legs produce (the handshake
        // and the request budgets do too) and matching it here is how this crate
        // would start retrying a rejected credential.
        classify_connect_error(error.kind(), &format!("Failed to connect to NATS: {error}"))
    })?;

    let identity = creds
        .secure
        .as_ref()
        .ok_or_else(|| crate::Error::Message("peer_pairing_identity_missing".to_string()))?;
    let private = URL_SAFE_NO_PAD
        .decode(&identity.private_key)
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    let peer_key = identity
        .peer_public_key
        .as_ref()
        .ok_or_else(|| crate::Error::Message("peer_pairing_identity_missing".to_string()))
        .and_then(|key| {
            URL_SAFE_NO_PAD
                .decode(key)
                .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))
        })?;
    let prologue = future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id)
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;

    // Pairing first when the invitation secret is still held; otherwise (and
    // after a pairing that already succeeded) `IK` against the pinned key.
    if let Some(secret) = identity.secret.as_deref() {
        match handshake(
            &client,
            creds,
            &private,
            &peer_key,
            Some(secret),
            &prologue,
            true,
            environment,
        )
        .await
        {
            Ok(session) => {
                return Ok(Connected {
                    session,
                    consumed_invitation: true,
                })
            }
            // The response to a *successful* pairing can be lost, and the host
            // deletes its PSK the moment it binds this identity — so a retry
            // with the PSK would fail forever while the pairing is in fact
            // valid. `IK` proves the same local identity against the pinned
            // key; it is not a TOFU or plaintext fallback.
            Err(error) => {
                future_remote_protocol::remote_log!(
                    "remote_peer: pairing handshake failed ({error}); retrying with IK"
                );
            }
        }
    }
    let session = handshake(
        &client,
        creds,
        &private,
        &peer_key,
        None,
        &prologue,
        false,
        environment,
    )
    .await?;
    Ok(Connected {
        session,
        consumed_invitation: false,
    })
}

/// One handshake exchange. `pairing` selects `Noise_XXpsk0` (with `secret`);
/// otherwise `Noise_IK` against the pinned peer key.
#[allow(clippy::too_many_arguments)]
async fn handshake(
    client: &async_nats::Client,
    creds: &PeerCreds,
    private: &[u8],
    peer_key: &[u8],
    secret: Option<&str>,
    prologue: &[u8],
    pairing: bool,
    environment: &ConnectionOptions,
) -> Result<PeerSession, crate::Error> {
    let secret_bytes: Option<[u8; 32]> = match secret {
        Some(secret) => Some(
            URL_SAFE_NO_PAD
                .decode(secret)
                .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?
                .try_into()
                .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?,
        ),
        None => None,
    };
    let pattern = if pairing {
        future_remote_crypto::Pattern::Pair
    } else {
        future_remote_crypto::Pattern::Reconnect
    };
    let mut noise = future_remote_crypto::Handshake::new(
        pattern,
        true,
        private,
        // XXpsk0 learns the responder's key during the handshake and checks it
        // against the invitation afterwards; IK requires it up front.
        (!pairing).then_some(peer_key),
        secret_bytes.as_ref(),
        prologue,
    )
    .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;

    let subject = format!("p.{}.cmd.handshake", creds.pair_id);
    let first = noise
        .write(b"")
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    let mut response = exchange(
        client,
        &subject,
        json!({
            "type": "secure_open",
            "pairing": pairing,
            "message": URL_SAFE_NO_PAD.encode(&first),
        }),
    )
    .await?;
    let message = URL_SAFE_NO_PAD
        .decode(
            response
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    noise
        .read(&message)
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    // The host must be the machine the invitation named. Checked here, before
    // the final message is sent, so a swapped peer never receives our half.
    if noise
        .remote_public_key()
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?
        != peer_key
    {
        return Err(crate::Error::Message(
            "remote_secure_channel_invalid".into(),
        ));
    }
    if pairing {
        let second = noise
            .write(b"")
            .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
        response = exchange(
            client,
            &subject,
            json!({
                "type": "secure_finish",
                "id": response.get("id").and_then(Value::as_str).unwrap_or_default(),
                "message": URL_SAFE_NO_PAD.encode(&second),
            }),
        )
        .await?;
    }
    let mut channel = noise
        .finish()
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    let encrypted = URL_SAFE_NO_PAD
        .decode(
            response
                .get("confirmation")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    let confirmation = channel
        .open("handshake-confirm", &encrypted)
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    let confirmation: Value = serde_json::from_slice(&confirmation)
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    let confirmed = Confirmed::from_confirmation(&confirmation, &creds.pair_id)?;

    let control = std::sync::Arc::new(std::sync::Mutex::new(Control::from_presence(
        &confirmed.presence,
    )));
    let session = PeerSession {
        pair_id: creds.pair_id.clone(),
        client: client.clone(),
        channel: std::sync::Arc::new(std::sync::Mutex::new(channel)),
        bridge_instance_id: confirmed.bridge_instance_id,
        features: confirmed.features,
        presence: confirmed.presence,
        control,
    };
    // What this client can consume. `secure_ready` is what commits the host's
    // candidate channel, so it must still be sent; the declared set now asks
    // for the lanes the host already implements and this client is audited for:
    //
    // * `event_coalescing_v1` — the merged event's `data.text` is the full
    //   concatenation of its fragment range, so a consumer with no index cursor
    //   (this one, and the TS timeline that dedups on `eventId`) renders it
    //   identically to the per-token lane. No consumer change is required.
    // * `reply_gzip_v1` — safe only because the reply decode path sniffs the
    //   gzip magic and inflates under a bounded cap (`super::codec`); declaring
    //   it without that reader would make every reply ≥ 32 KiB undecodable.
    // * `selective_events_v1` — negotiated, and only acted on when the host
    //   confirms it (`session::Subscriptions`).
    // * `lean_events_v1` — what it strips has a fallback now: `sessionId` stays
    //   in the trimmed body (the `state.events` mirror's subject names no
    //   conversation) and is recovered from the subject, and the live entry
    //   falls back to `data` and then `{runId}:{idx}`. Paired with the
    //   `get_tool_call_args` read the host serves only to a client that declared
    //   it, which is what keeps a lean shell row's command on screen.
    //
    // It also carries this client's identity, which is not a capability: the host
    // shows the user what is connected, and it cannot say "a computer" about
    // something that never told it so. An older host ignores both fields.
    session
        .request(
            &session.command_subject("handshake"),
            json!({
                "type": "secure_ready",
                "features": declared_features(&session.features),
                "deviceName": environment.device_name,
                "deviceKind": environment.device_kind,
            }),
            handshake_budget(),
        )
        .await?;
    Ok(session)
}

/// Subscribe to one subject, with the failure classified the same way for both
/// streams.
///
/// One implementation rather than two identical `map_err`s: the second one would
/// only ever be reachable by the call that happens to run first, so it could
/// never be exercised.
async fn subscribe(
    client: &async_nats::Client,
    subject: String,
) -> Result<async_nats::Subscriber, crate::Error> {
    client
        .subscribe(subject)
        .await
        .map_err(|error| crate::Error::RemoteTransport(format!("Remote subscribe failed: {error}")))
}

/// One unencrypted handshake leg, exercised directly by a test.
///
/// The only way to make a handshake fail is against a socket the test itself
/// controls: a broker that accepts a connection and then stops answering is not
/// something the fake broker can be scripted into mid-handshake.
#[cfg(any(test, feature = "test-support"))]
pub async fn exchange_for_test(
    client: &async_nats::Client,
    subject: &str,
    body: Value,
) -> Result<Value, crate::Error> {
    exchange(client, subject, body).await
}

/// One unencrypted handshake leg. Every field this client reads is optional by
/// construction — the host answers refusals with `success: false` — so a
/// missing field is reported as a protocol error rather than a panic.
async fn exchange(
    client: &async_nats::Client,
    subject: &str,
    body: Value,
) -> Result<Value, crate::Error> {
    let bytes = serde_json::to_vec(&body)
        .map_err(|error| crate::Error::Message(format!("encode handshake: {error}")))?;
    let response = tokio::time::timeout(
        handshake_budget(),
        client.request(subject.to_string(), bytes.into()),
    )
    .await
    .map_err(|_| crate::Error::RemoteTransport("Remote handshake timed out".into()))?
    .map_err(|error| crate::Error::RemoteTransport(format!("Remote handshake failed: {error}")))?;
    let reply: Value = serde_json::from_slice(&response.payload)
        .map_err(|_| crate::Error::Message("remote_secure_channel_invalid".into()))?;
    if reply.get("success").and_then(Value::as_bool) != Some(true) {
        // The host appends *why* after the stable token (`invitation`, `peer`,
        // …). Keeping it is what turns "pairing failed" into an actionable
        // message on both sides.
        let detail = reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(crate::Error::Message(format!(
            "remote_secure_channel_invalid ({detail})"
        )));
    }
    Ok(reply.get("data").cloned().unwrap_or(Value::Null))
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    #[test]
    fn command_subjects_use_the_hosts_two_lane_names() {
        assert_eq!(command_subject("pair_1", ""), "p.pair_1.cmd.new");
        assert_eq!(command_subject("pair_1", "list"), "p.pair_1.cmd.list");
        assert_eq!(command_subject("pair_1", "sess_9"), "p.pair_1.cmd.sess_9");
    }

    /// The declaration is why the host sends gzip replies, coalesced events and
    /// trimmed ones at all, and why the narrow event lane may be used: dropping
    /// any entry silently reverts this client to the legacy lanes. Asserted as a
    /// set (order is free) and as the wire shape, because a declaration the host
    /// cannot read is the same as none.
    ///
    /// `lean_events_v1` is asserted **present** now, and that assertion is the one
    /// that has to be earned: everything it strips has a fallback (the trimmed
    /// body keeps `sessionId`, the subject is recovered into the payload, and the
    /// live entry falls back to `data` then `{runId}:{idx}`), and the
    /// `get_tool_call_args` read it unlocks is what keeps a shell row's command on
    /// screen. Removing a fallback without removing this entry is what this test
    /// exists to stop.
    ///
    /// `catalog_activity_v1` is earned the same way: asking for it stops the host
    /// re-sending a whole session list per second of every run, so it is only
    /// truthful while `useRemotePeers` applies the packet that replaces it —
    /// which is the *only* thing that moves a row on this side. Drop that applier
    /// without dropping this entry and the merged list stops re-ordering.
    ///
    /// `client_presence_v1` is a promise rather than a lane: the host reads a
    /// lapsed beat as a departure, so it must only be declared while
    /// `spawn_presence_beats` is actually beating on this connection. A client
    /// that declared it and then went quiet would be reported gone while it was
    /// sitting there.
    ///
    /// `read_binary_v1` is the only *negotiated* entry: it is declared only when
    /// the host's confirmation advertised it ([`declared_features`]), because it
    /// changes how a reply is encoded rather than what it contains. The
    /// fallback is the base64 page this client has always read, and it is what
    /// every host that does not advertise the flag keeps sending.
    #[test]
    fn the_client_declares_the_lanes_its_decoders_expect() {
        for expected in [
            "event_coalescing_v1",
            "reply_gzip_v1",
            SELECTIVE_EVENTS_FEATURE,
            future_remote_protocol::LEAN_EVENTS_FEATURE,
            CATALOG_ACTIVITY_FEATURE,
            future_remote_protocol::CLIENT_PRESENCE_FEATURE,
            future_remote_protocol::READ_BINARY_FEATURE,
            future_remote_protocol::SEGMENTED_CONTENT_FEATURE,
        ] {
            assert!(
                DECLARED_FEATURES.contains(&expected),
                "the handshake must declare {expected}: {DECLARED_FEATURES:?}"
            );
        }
        let legacy = [
            "event_coalescing_v1",
            "segmented_content_v1",
            "reply_gzip_v1",
            "selective_events_v1",
            "lean_events_v1",
            "catalog_activity_v1",
            "client_presence_v1",
        ];
        let binary = future_remote_protocol::READ_BINARY_FEATURE;
        // A host that does not advertise the frame gets exactly the declaration
        // this client sent before the capability existed: no new entry, no
        // silently dropped old one.
        assert_eq!(
            json!({ "features": declared_features(&[]) })["features"],
            json!(legacy)
        );
        // An unrelated capability in the confirmation does not enable it either.
        assert_eq!(
            json!({ "features": declared_features(&[SELECTIVE_EVENTS_FEATURE.to_string()]) })
                ["features"],
            json!(legacy)
        );
        let mut confirmed = vec![binary.to_string()];
        confirmed.extend(legacy.iter().map(|feature| feature.to_string()));
        assert_eq!(
            json!({ "features": declared_features(&confirmed) })["features"],
            json!([
                "event_coalescing_v1",
                "segmented_content_v1",
                "reply_gzip_v1",
                "selective_events_v1",
                "lean_events_v1",
                "catalog_activity_v1",
                "client_presence_v1",
                "read_binary_v1"
            ])
        );
    }

    /// Which lane an installed events subscription is on.
    ///
    /// This is the decision the whole capability turns on, and both halves of it
    /// are silent failures when wrong: narrowing for a host that never confirmed
    /// the flag leaves the transcript empty, and narrowing for an id that is not
    /// a plain token widens the subscription back to the whole host (`*`) or
    /// addresses a subject the host never publishes on (`.`).
    #[test]
    fn the_event_lane_narrows_only_for_a_confirmed_plain_session() {
        let narrow = events_lane("pair_1", true, Some("sess_a"));
        assert_eq!(narrow.subject, "p.pair_1.evt.sess_a");
        assert!(narrow.narrow);

        // Nothing is being viewed: the lane this client has always used.
        for (confirmed, visible) in [
            (true, None),
            // A host that did not confirm the capability keeps the wide lane,
            // whatever the UI is looking at.
            (false, Some("sess_a")),
            (false, None),
            // An empty id is "no conversation", not a subject.
            (true, Some("")),
        ] {
            let lane = events_lane("pair_1", confirmed, visible);
            assert_eq!(lane.subject, "p.pair_1.evt.>", "{confirmed:?} {visible:?}");
            assert!(!lane.narrow, "{confirmed:?} {visible:?}");
        }
    }

    /// An id that is not a plain token cannot narrow anything, and must not be
    /// interpolated into a subject: `*` and `>` are wildcards, so a session id
    /// carrying one would subscribe to *more* than the wide lane, and a `.`
    /// would address a subject the host never publishes on.
    #[test]
    fn a_wildcard_session_id_cannot_widen_the_subscription() {
        for hostile in ["*", ">", "sess.*", "a.b", "sess >", "sess_a;"] {
            assert_eq!(narrowable_session(hostile), None, "{hostile}");
            let lane = events_lane("pair_1", true, Some(hostile));
            assert_eq!(lane.subject, "p.pair_1.evt.>", "{hostile}");
            assert!(!lane.narrow, "{hostile}");
        }
        // The ids the host actually mints are accepted.
        for id in ["sess_a", "session-1_2", "0f6c4a1b"] {
            assert_eq!(narrowable_session(id), Some(id));
            assert_eq!(
                events_lane("pair_1", true, Some(id)).subject,
                format!("p.pair_1.evt.{id}")
            );
        }
    }

    /// The host-wide lane is a wildcard subscription, and it is the mirror the
    /// narrow lane depends on: the host publishes every cross-conversation
    /// notification there as well (`remote::publisher::is_catalog_event`).
    #[test]
    fn the_host_wide_state_lane_is_subscribed_by_wildcard() {
        // `state.>` rather than `state.events`, even though only `state.events`
        // is read: the wildcard is what the phone subscribes to, so it is the
        // shape the platform's subject permissions are known to allow.
        assert_eq!(state_subject("pair_1"), "p.pair_1.state.>");
    }

    /// Asserted here rather than through a live connection: whether an agent
    /// happens to be reachable is a property of the machine running the test,
    /// not of the client, so an end-to-end check of it would be asserting the
    /// environment. What the client owns is the *reading*.
    #[test]
    fn agent_availability_is_read_from_the_hosts_own_statement() {
        assert!(agent_available(&json!({ "agentAvailable": true })));
        assert!(!agent_available(&json!({ "agentAvailable": false })));
        // An older host that omits the field, or reports it as something else,
        // must not be treated as healthy: the UI says "everything is fine" on
        // `true`, and the user would otherwise find out from a failed command.
        assert!(!agent_available(&json!({})));
        assert!(!agent_available(&json!({ "agentAvailable": "yes" })));
        assert!(!agent_available(&Value::Null));
    }

    #[test]
    fn control_is_read_from_presence_and_silence_is_not_a_state() {
        assert_eq!(
            Control::from_presence(&json!({})),
            Control::default(),
            "an older host says nothing"
        );
        let mut control = Control::from_presence(&json!({ "agentAvailable": true }));
        assert_eq!(control.agent_available, Some(true));
        assert!(control.state.is_none() && control.holder.is_none());

        control.apply_presence(&json!({
            "control": "standby",
            "holder": { "pairId": "pair_phone", "name": "Phone", "kind": "mobile" },
        }));
        assert_eq!(control.state, Some(ControlState::Standby));
        let holder = control.holder.clone().expect("the holder is named");
        assert_eq!(holder.pair_id.as_deref(), Some("pair_phone"));
        assert_eq!(holder.name.as_deref(), Some("Phone"));
        assert_eq!(holder.kind.as_deref(), Some("mobile"));

        // The controller's own packet has no holder; a stale one must not
        // survive the move back.
        control.apply_presence(&json!({ "control": "active" }));
        assert_eq!(control.state, Some(ControlState::Active));
        assert_eq!(control.holder, None);

        // A word this client does not know, and a state that is not a string,
        // change nothing: the machine stays as it was rather than being
        // declared unusable by a newer host's vocabulary.
        control.apply_presence(&json!({ "control": "handover" }));
        assert_eq!(control.state, Some(ControlState::Active));
        control.apply_presence(&json!({ "control": 1 }));
        assert_eq!(control.state, Some(ControlState::Active));
    }

    /// A `state.control` packet names the state and not the holder.
    ///
    /// It arrives the moment control moves, which is *before* the presence tick
    /// that names the device that now has it. Keeping the previous holder would
    /// name the device that was just displaced — so the holder is cleared and
    /// the client says "another device" until the next tick says which.
    #[test]
    fn a_control_packet_clears_the_holder_it_cannot_name() {
        let mut control = Control::default();
        control.apply_presence(&json!({
            "control": "standby",
            "holder": { "name": "Old phone" },
        }));
        assert_eq!(
            control
                .holder
                .as_ref()
                .and_then(|holder| holder.name.clone()),
            Some("Old phone".to_string())
        );

        control.apply_control(&json!({ "control": "standby", "pairId": "pair_1" }));
        assert_eq!(control.state, Some(ControlState::Standby));
        assert_eq!(
            control.holder, None,
            "the displaced device is not the holder"
        );

        control.apply_control(&json!({ "control": "active" }));
        assert_eq!(control.state, Some(ControlState::Active));

        // A packet without a known state leaves the reading alone.
        control.apply_control(&json!({ "pairId": "pair_1" }));
        assert_eq!(control.state, Some(ControlState::Active));
    }

    fn confirmation(overrides: Value) -> Value {
        let mut base = json!({
            "confirmed": true,
            "pairId": "pair_1",
            "bridgeInstanceId": "bridge-1",
            "features": ["e2ee_v2"],
            "presence": { "agentAvailable": true },
        });
        // A non-object override cannot be merged; the cases below are all
        // objects, and asserting that keeps the helper from silently ignoring
        // one that is not.
        let target = base.as_object_mut().expect("a JSON object literal");
        for (key, value) in overrides.as_object().expect("object overrides") {
            target.insert(key.clone(), value.clone());
        }
        base
    }

    #[test]
    fn a_well_formed_confirmation_is_accepted() {
        let confirmed = Confirmed::from_confirmation(&confirmation(json!({})), "pair_1")
            .expect("valid confirmation");
        assert_eq!(confirmed.bridge_instance_id, "bridge-1");
        assert_eq!(confirmed.features, vec!["e2ee_v2".to_string()]);
        assert_eq!(confirmed.presence["agentAvailable"], json!(true));
    }

    /// Every case here is a protocol fault no real host produces, so handing the
    /// validator what a buggy or hostile one would send is the only way to reach
    /// the arms — and they are exactly the arms that decide whether a client
    /// trusts a claimed identity.
    #[test]
    fn a_confirmation_that_does_not_prove_this_pairing_is_refused() {
        let cases = [
            // A relay replaying another pairing's confirmation.
            ("another pair", json!({ "pairId": "pair_other" })),
            ("missing pair", json!({ "pairId": null })),
            // A host that never affirmed the handshake.
            ("not confirmed", json!({ "confirmed": false })),
            ("confirmed missing", json!({ "confirmed": null })),
            // No bridge identity: nothing to tell one host incarnation from another.
            ("no bridge", json!({ "bridgeInstanceId": null })),
            ("empty bridge", json!({ "bridgeInstanceId": "" })),
            // A peer that cannot speak the record layer we are about to use.
            ("no features", json!({ "features": null })),
            ("other features", json!({ "features": ["something_else"] })),
            ("features wrong type", json!({ "features": "e2ee_v2" })),
        ];
        for (label, overrides) in cases {
            let result = Confirmed::from_confirmation(&confirmation(overrides), "pair_1");
            assert!(result.is_err(), "{label} must be refused, but was accepted");
            assert_eq!(
                result.err().map(|error| error.to_string()).as_deref(),
                Some("remote_secure_channel_invalid"),
                "{label}"
            );
        }
    }

    /// A host that reports no presence at all is still usable: only the fields
    /// the client actually relies on are required.
    #[test]
    fn a_missing_presence_is_not_a_protocol_fault() {
        let confirmed =
            Confirmed::from_confirmation(&confirmation(json!({ "presence": null })), "pair_1")
                .expect("presence is optional");
        assert_eq!(confirmed.presence, Value::Null);
        assert!(!agent_available(&confirmed.presence));
    }

    /// A refused credential and a dead network must arrive as *different*
    /// variants, because the runtime's decision to retry is made on the variant.
    ///
    /// The two arms are asserted directly on the classifier rather than through a
    /// live broker: `FakeNats` cannot be made to refuse authorization, and a test
    /// that cannot fail is worse than no test. The library's own kinds are used,
    /// not message text — several legs say "timed out", and matching that phrase
    /// is how the crate would start retrying a rejected credential.
    #[test]
    fn connect_failures_are_classified_by_their_kind() {
        use async_nats::ConnectErrorKind;
        // The credential is wrong: permanent, and the user must re-pair.
        for kind in [
            ConnectErrorKind::Authentication,
            ConnectErrorKind::AuthorizationViolation,
        ] {
            let error = classify_connect_error(kind, "Failed to connect to NATS: x");
            assert!(
                matches!(error, crate::Error::RemoteAuthorization(_)),
                "{kind:?} must not be retried"
            );
        }
        // Everything else on this leg is the network: retryable.
        for kind in [
            ConnectErrorKind::TimedOut,
            ConnectErrorKind::Io,
            ConnectErrorKind::Dns,
            ConnectErrorKind::MaxReconnects,
        ] {
            let error = classify_connect_error(kind, "Failed to connect to NATS: x");
            assert!(
                matches!(error, crate::Error::RemoteTransport(_)),
                "{kind:?} is the transient class"
            );
        }
        // The message is carried through unchanged, so the UI shows the same text
        // it always did.
        assert_eq!(
            classify_connect_error(ConnectErrorKind::TimedOut, "Failed to connect to NATS: t")
                .to_string(),
            "Failed to connect to NATS: t"
        );
    }

    /// The production budgets are "the phone's" and "the library's default",
    /// and they are asserted **from the test build** — which is the only thing
    /// that makes the fixture tolerance safe.
    ///
    /// A fixture tolerance for these budgets was measured and withdrawn
    /// (see `request_budget`), but the lesson stays: a budget a test cannot see
    /// is a budget that can drift. These constants are the numbers themselves —
    /// the wire contract they encode is the phone's (`mobile/src/remote/
    /// client.ts` sends `{ timeout: 10_000 }`) — so the pin belongs here rather
    /// than in a comment, together with the wiring that reads them.
    #[test]
    fn production_budgets_are_the_phone_and_the_library() {
        assert_eq!(
            PHONE_REQUEST_TIMEOUT,
            Duration::from_secs(10),
            "the phone sends {{ timeout: 10_000 }} on the command and handshake lanes"
        );
        assert_eq!(
            HANDSHAKE_TIMEOUT, PHONE_REQUEST_TIMEOUT,
            "the handshake is one of those requests"
        );
        assert_eq!(
            NATS_CONNECT_TIMEOUT,
            Duration::from_secs(5),
            "async-nats 0.49's own ConnectOptions::connection_timeout default, now written down"
        );
        // And the wiring: what a command and a socket actually get is the pair
        // above, in this build too — the one assertion that would have caught a
        // fixture tolerance being left in place unconditionally.
        assert_eq!(COMMAND_TIMEOUT, PHONE_REQUEST_TIMEOUT);
        assert_eq!(handshake_budget(), HANDSHAKE_TIMEOUT);
        assert_eq!(request_budget(), PHONE_REQUEST_TIMEOUT);
        assert_eq!(connect_budget(), NATS_CONNECT_TIMEOUT);
    }
}

#[cfg(test)]
mod retry_policy_tests {
    use super as session;
    use serde_json::json;

    /// Which commands the retry may touch, without a connection: the whitelist is a
    /// property of the command, and these two lists are the whole contract. Both
    /// halves matter — a read left out is a self-healing path that never heals, and
    /// a write let in is a duplicated side effect.
    #[test]
    fn presence_updates_agent_health_without_changing_legacy_control() {
        let mut control = session::Control::from_presence(&json!({"agentAvailable": false}));
        assert_eq!(control.agent_available, Some(false));
        assert_eq!(control.state, None);
        control.apply_presence(&json!({"agentAvailable": true}));
        assert_eq!(control.agent_available, Some(true));
        assert_eq!(control.state, None);
        control.apply_presence(&json!({"control": "active"}));
        assert_eq!(control.agent_available, Some(true));
        assert_eq!(control.state, Some(session::ControlState::Active));
    }

    #[test]
    fn conversation_retries_require_receipts_and_a_stable_identity() {
        let features = vec!["prompt_receipt_v1".to_string()];
        for kind in ["prompt", "continue_run"] {
            assert!(!session::retryable_for_features(
                &json!({"id":"request", "type":kind}),
                &[]
            ));
            assert!(!session::retryable_for_features(
                &json!({"type":kind}),
                &features
            ));
            assert!(session::retryable_for_features(
                &json!({"id":"request", "type":kind}),
                &features
            ));
        }
    }

    #[test]
    fn only_reads_and_the_two_conversation_commands_are_retryable() {
        for kind in [
            "list_sessions",
            "list_workspaces",
            "get_state",
            "get_read_chunk",
            "upload_complete",
            "prompt",
            "continue_run",
        ] {
            assert!(
                session::retryable_command(&json!({ "type": kind })),
                "{kind} must be retryable"
            );
        }
        for kind in [
            "create_workspace",
            "set_task_enabled",
            "upload_init",
            "upload_cancel",
            "download_prepare",
            "abort",
            "approval_decision",
            "secure_ready",
        ] {
            assert!(
                !session::retryable_command(&json!({ "type": kind })),
                "{kind} must not be retried"
            );
        }
        assert!(
            !session::retryable_command(&json!({})),
            "a body with no command type is not a command"
        );
    }
}
