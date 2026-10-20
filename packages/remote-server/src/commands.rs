//! Authenticated command admission, single-flight replies and role routing.
//! Local business and runtime context are supplied explicitly by the owner.

use crate::services::{BusinessHost, ReplySink};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use future_remote_protocol::IncomingCmd;
use futures::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

static COMMAND_EPISODE: LazyLock<crate::diagnostics::FailureEpisode> =
    LazyLock::new(Default::default);
/// Refused handshakes. Any peer holding the pair's NATS JWT can attempt one, so
/// this goes through the same quota mechanism as every other bridge fault
/// rather than printing once per frame.
static HANDSHAKE_EPISODE: LazyLock<crate::diagnostics::FailureEpisode> =
    LazyLock::new(Default::default);

fn remote_json_gzip_enabled() -> bool {
    reply_gzip_allowed()
        || COMMAND_RUNTIME
            .try_with(|runtime| runtime.gzip_override())
            .unwrap_or(false)
}

struct CachedReply {
    request: Value,
    response: Arc<tokio::sync::Mutex<Option<Vec<u8>>>>,
}

type ReplySlot = Arc<CachedReply>;

/// Command-id → in-flight/completed response cache (single-flight). Created
/// once per bridge start and SHARED across command loops: credential refresh
/// swaps the loop every JWT TTL, and a cache local to the loop would be wiped
/// on every swap — a client retrying right after a swap would re-execute a
/// command the old loop had already run (for `prompt`, a duplicated message).
#[derive(Clone, Default)]
pub struct ReplySlots(Arc<Mutex<HashMap<String, ReplySlot>>>);
impl ReplySlots {
    /// Read-only observation of real cache retirement by integration fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn contains_command(&self, command_id: &str) -> bool {
        self.0.lock().unwrap().contains_key(command_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn shares_cache(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone)]
pub struct HandshakeState {
    pub secure: crate::secure::Transport,
    host: Arc<dyn BusinessHost>,
    runtime: Arc<dyn CommandRuntime>,
    reply_ttl: Duration,
    creds: crate::PairingCreds,
    access_epoch: Option<u64>,
    confirmed: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    bridge_instance_id: String,
    pending: Arc<Mutex<HashMap<String, PendingHandshake>>>,
    /// Whether this connection's replies may be gzipped, set when the client
    /// declares it on `secure_ready`. It lives with the handshake — not the
    /// process — because an older client that never declares it must keep
    /// receiving plain JSON: it has no magic-byte detection and would fail to
    /// parse a compressed reply.
    pub gzip_replies: Arc<AtomicBool>,
    /// Whether this connection declared the lean feed.
    ///
    /// Lives with the connection like [`HandshakeState::gzip_replies`]: it is a
    /// property of who is asking, and this machine now serves several clients,
    /// so a process-wide flag would trim one device's replies to another
    /// device's declaration.
    pub lean_events: Arc<AtomicBool>,
}

/// Capability a client declares on `secure_ready` to accept gzip-compressed
/// command replies. Additive: a client that does not ask never receives one.
pub use future_remote_protocol::REPLY_GZIP_FEATURE;

/// Record what the client said it was.
///
/// Both fields are optional on the wire, and a client that sends neither is left
/// as "unknown" rather than described wrongly: the UI has wording for an
/// unnamed device, and none for a device it mislabels. An empty declaration
/// therefore clears a previous one — a client that stops describing itself must
/// not keep the old description on screen.
fn record_client_identity(runtime: &dyn CommandRuntime, pair_id: &str, command: &IncomingCmd) {
    // A pairing with no live runtime has nowhere to record this, and the
    // accessor hands back a throwaway slot for an unknown one. Writing into that
    // would look like it worked while the status kept saying "unknown", so the
    // absence is checked here.
    let Some(slot) = runtime.client_identity(pair_id) else {
        return;
    };
    *slot.lock().unwrap() = declared_identity(command);
}

/// What a `secure_ready` declared, normalised.
///
/// Pure, so every rule is testable without a running bridge: a name is trimmed,
/// a blank one means "did not say", and the kind is kept verbatim because it is
/// opaque to this side — an unknown kind is a rendering question for the UI, not
/// a reason to discard the name it came with.
fn declared_identity(command: &IncomingCmd) -> Option<crate::types::ClientIdentity> {
    let name = command.device_name.trim();
    if name.is_empty() {
        return None;
    }
    Some(crate::types::ClientIdentity {
        name: name.to_string(),
        kind: command.device_kind.trim().to_string(),
    })
}

/// Record what a connection declared on `secure_ready`. Every capability is
/// opt-in, and the declared set is authoritative: a declaration always writes
/// the flag, so an empty list clears a previous declaration rather than leaving
/// it latched.
pub fn apply_declared_features(handshake: &HandshakeState, pair_id: &str, features: &[String]) {
    handshake.runtime.coalesce_events(pair_id).store(
        features
            .iter()
            .any(|feature| feature == future_remote_protocol::EVENT_COALESCING_FEATURE),
        Ordering::Release,
    );
    handshake.runtime.catalog_activity(pair_id).store(
        features
            .iter()
            .any(|feature| feature == future_remote_protocol::CATALOG_ACTIVITY_FEATURE),
        Ordering::Release,
    );
    // The declaration belongs to the connection that made it: it is recorded on
    // the handshake (what the command path reads) and on the pairing's shared
    // runtime (what the event publisher reads, since events go to the device in
    // control). This machine serves several clients, so neither may be global.
    let lean = crate::lean::feature_declared(features);
    handshake.lean_events.store(lean, Ordering::Release);
    handshake
        .runtime
        .lean_events(pair_id)
        .store(lean, Ordering::Release);
    handshake.gzip_replies.store(
        features.iter().any(|feature| feature == REPLY_GZIP_FEATURE),
        Ordering::Release,
    );
    // The promise that makes silence mean something. A client that declares it
    // will keep beating while it is there, so a lapsed beat is a departure; a
    // client that does not is only ever retired by its own goodbye.
    if let Some(link) = handshake.runtime.client_link(pair_id) {
        link.declare(
            features
                .iter()
                .any(|feature| feature == future_remote_protocol::CLIENT_PRESENCE_FEATURE),
        );
    }
}

#[derive(Clone)]
struct PendingHandshake {
    transcript: String,
    device_id: String,
    client_public_key: String,
    created_at: std::time::Instant,
}

impl HandshakeState {
    pub(crate) fn new(
        creds: crate::PairingCreds,
        confirmed: Arc<AtomicBool>,
        bridge_instance_id: String,
        services: CommandServices,
    ) -> Self {
        Self {
            secure: crate::secure::Transport::new(&creds, services.runtime.clone()),
            creds,
            access_epoch: None,
            host: services.business,
            runtime: services.runtime,
            reply_ttl: services.reply_ttl,
            confirmed,
            active: Arc::new(AtomicBool::new(false)),
            bridge_instance_id,
            pending: Arc::new(Mutex::new(HashMap::new())),
            gzip_replies: Arc::new(AtomicBool::new(false)),
            lean_events: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_access(mut self, epoch: u64) -> Self {
        self.access_epoch = Some(epoch);
        self
    }

    pub(crate) fn access_epoch(&self) -> Option<u64> {
        self.access_epoch
    }

    fn access_current(&self) -> bool {
        self.access_epoch
            .is_none_or(|epoch| self.runtime.access().current() == epoch)
    }

    pub fn bridge_instance_id(&self) -> &str {
        &self.bridge_instance_id
    }

    pub fn active_flag(&self) -> Arc<AtomicBool> {
        self.active.clone()
    }
}

pub fn new_reply_slots() -> ReplySlots {
    ReplySlots::default()
}

tokio::task_local! {
    static REPLY_CAPTURE: Arc<Mutex<Option<Vec<u8>>>>;
}

tokio::task_local! {
    /// Whether the connection serving this command accepts gzipped replies.
    /// A task-local keeps the flag on the reply path without threading it
    /// through every handler between the command loop and `reply`.
    static REPLY_GZIP: bool;
}

/// Read the reply-encoding permission, defaulting to off outside a command.
fn reply_gzip_allowed() -> bool {
    REPLY_GZIP.try_with(|allowed| *allowed).unwrap_or(false)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn command_loop(
    client: async_nats::Client,
    pair_id: String,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
) {
    command_loop_with_ready(client, pair_id, reply_slots, handshake, None).await;
}

pub async fn command_loop_with_ready(
    client: async_nats::Client,
    pair_id: String,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
    mut ready: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let subject = format!("p.{pair_id}.cmd.>");
    let queue = format!("bridge.{pair_id}");
    let mut sub = match client.queue_subscribe(subject, queue).await {
        Ok(sub) => sub,
        Err(error) => {
            if let Some(line) = COMMAND_EPISODE.record("command_subscription", error) {
                eprintln!("{line}");
            }
            return;
        }
    };
    if let Some(line) = COMMAND_EPISODE.recovered() {
        eprintln!("{line}");
    }
    if let Some(sender) = ready.take() {
        let _ = sender.send(());
    }
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let reply_slots = reply_slots.clone();
        let handshake = handshake.clone();
        let pair_id = pair_id.clone();
        // Spawn per command: prevent a slow command from blocking others.
        let runtime = handshake.runtime.clone();
        let context = runtime.clone();
        runtime.spawn(Box::pin(COMMAND_RUNTIME.scope(context, async move {
            if handshake.secure.enabled() {
                if !handshake.access_current() {
                    return;
                }
                if msg.payload.starts_with(b"FRE2") {
                    let Ok((plain, security)) = handshake.secure.open(&msg.subject, &msg.payload)
                    else {
                        return;
                    };
                    let mut msg = msg;
                    msg.payload = plain.into();
                    // Handshake messages are never business commands, even
                    // when a paired peer sends them inside a secure record.
                    let parsed = serde_json::from_slice::<IncomingCmd>(&msg.payload);
                    if parsed.as_ref().is_ok_and(|c| {
                        matches!(
                            c.cmd_type.as_str(),
                            "pair_handshake" | "pair_handshake_confirm"
                        )
                    }) {
                        return;
                    }
                    if parsed.as_ref().is_ok_and(|c| c.cmd_type == "secure_ready") {
                        // Declared capabilities are per connection: a client that
                        // asks for the coalesced lane gets it, and every other
                        // client keeps the legacy one-event-per-token lane.
                        if let Ok(command) = parsed.as_ref() {
                            apply_declared_features(&handshake, &pair_id, &command.features);
                            record_client_identity(handshake.runtime.as_ref(), &pair_id, command);
                        }
                        // The sealed `secure_ready` is the client saying it is
                        // here on the channel it will keep beating on.
                        if let Some(link) = handshake.runtime.client_link(&pair_id) {
                            link.observe();
                        }
                        let activate = || {
                            handshake.secure.activate(&security)?;
                            handshake.active.store(true, Ordering::Release);
                            Ok::<_, crate::Error>(())
                        };
                        let activated = match handshake.access_epoch {
                            Some(epoch) => handshake.runtime.access().commit(epoch, activate),
                            None => Some(activate()),
                        };
                        if matches!(activated, Some(Ok(()))) {
                            crate::secure::REPLY
                                .scope(security, reply(&client, &msg, true, json!({}), None))
                                .await;
                        }
                        return;
                    }
                    if !handshake.secure.is_active(&security) {
                        return;
                    }
                    crate::secure::REPLY
                        .scope(
                            security,
                            handle_command_singleflight(&client, msg, reply_slots, handshake),
                        )
                        .await;
                } else {
                    // Whether this is the pairing's first handshake — the one
                    // that puts the device in front of the machine. Read before
                    // the commit, acted on after it: the access epoch's critical
                    // section is for deciding *whether* a result installs, and
                    // it must stay short. Persisting a control change is file
                    // work, and a handshake that held that lock while writing
                    // would stall every other connection's handshake behind it.
                    let first_handshake = !handshake.confirmed.load(Ordering::Acquire);
                    let process = || {
                        let result = handshake
                            .secure
                            .handshake(&msg.payload, &handshake.bridge_instance_id);
                        if let Ok(data) = &result {
                            if data.get("confirmation").is_some() {
                                handshake.confirmed.store(true, Ordering::Release);
                            }
                        }
                        result
                    };
                    let result = match handshake.access_epoch {
                        Some(epoch) => match handshake.runtime.access().commit(epoch, process) {
                            Some(result) => result,
                            None => return,
                        },
                        None => process(),
                    };

                    // A refused handshake is the one pairing failure that used to
                    // leave no trace anywhere: the phone only reports that the
                    // desktop would not authenticate, and the reason never left
                    // this function. Log it, and send it to the phone (which has
                    // the console a user can actually be asked to read).
                    if let Err(error) = &result {
                        if let Some(line) = HANDSHAKE_EPISODE.record("handshake_refused", error) {
                            eprintln!("{line}");
                        }
                    }
                    let accepted = result.is_ok();
                    if accepted {
                        // A successful handshake is the client proving it is
                        // there: the pairing's keys are the only way to reach
                        // this branch.
                        if let Some(link) = handshake.runtime.client_link(&handshake.creds.pair_id)
                        {
                            link.observe();
                        }
                    }
                    if let Some(reply) = msg.reply {
                        let body = match result {
                            Ok(data) => json!({ "success": true, "data": data }),
                            Err(error) => json!({ "success": false, "error": error.to_string() }),
                        };
                        let _ = client
                            .publish(reply, serde_json::to_vec(&body).expect("JSON value").into())
                            .await;
                    }
                    // Control moves *after* the answer, never before it: the
                    // claim persists to disk, and a handshake reply must not
                    // wait on a file write. The device is already authenticated
                    // by this point, so the few milliseconds in between belong
                    // to nobody's authority.
                    //
                    // …and only when the machine is free. Connecting is not
                    // taking over: if another device holds the machine, this
                    // one comes up in standby, reads `control`/`holder` from its
                    // presence, and asks its user whether to displace the
                    // controller. The client decides that, not this machine.
                    //
                    // A *first* handshake is one claim path (a device was just
                    // paired). The other is a reconnect onto a machine nobody
                    // holds while this is the only client here — a device that
                    // disconnected gave the machine up, and it is not asking its
                    // user to take back a computer it alone is sitting at.
                    let alone_and_unowned = handshake
                        .runtime
                        .alone_and_unowned(&handshake.creds.pair_id);
                    if (first_handshake || alone_and_unowned)
                        && handshake.access_current()
                        && accepted
                        && !handshake.runtime.held_by_another(&handshake.creds.pair_id)
                    {
                        let moved = handshake
                            .runtime
                            .claim_control_soon(&handshake.creds.pair_id, handshake.access_epoch);
                        if !handshake.access_current() {
                            return;
                        }
                        if let Some(previous) = moved {
                            handshake.runtime.publish_control(&previous, "standby");
                        }
                        handshake
                            .runtime
                            .publish_control(&handshake.creds.pair_id, "active");
                    }
                }
                #[cfg(any(test, feature = "test-support"))]
                return;
            }
            // Legacy protocol fixtures are retained solely for business-route
            // unit tests. Production must never accept a v1 connection.
            #[cfg(any(test, feature = "test-support"))]
            handle_command_singleflight(&client, msg, reply_slots, handshake).await;
        })));
    }
    if let Some(line) =
        COMMAND_EPISODE.record("command_subscription", "subscription ended unexpectedly")
    {
        eprintln!("{line}");
    }
}

/// Merge concurrent/retried deliveries carrying the same command id. The first
/// delivery executes the command; followers wait for and receive the exact same
/// response bytes. Completed responses stay cached for ten minutes, matching
/// the planned NATS duplicate window, then expire without blocking unrelated ids.
async fn handle_command_singleflight(
    client: &async_nats::Client,
    msg: async_nats::Message,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
) {
    if !handshake.access_current() {
        return;
    }
    let command = serde_json::from_slice::<IncomingCmd>(&msg.payload).ok();
    // Chunk reads are already idempotent against the bounded immutable cache.
    // Do not retain a second copy of every chunk in the ten-minute command
    // reply cache. They still pass the normal handshake/access checks below.
    if command
        .as_ref()
        .is_some_and(|cmd| cmd.cmd_type == "get_read_chunk")
    {
        handle_command(client, msg, handshake).await;
        return;
    }
    let command_id = command.map(|cmd| cmd.id).filter(|id| !id.is_empty());
    let Some(command_id) = command_id else {
        handle_command(client, msg, handshake).await;
        return;
    };

    let request: Value = serde_json::from_slice(&msg.payload).expect("command already decoded");
    let (slot, inserted) = {
        let mut slots = reply_slots.0.lock().unwrap();
        match slots.get(&command_id) {
            Some(slot) => (slot.clone(), false),
            None => {
                let slot = Arc::new(CachedReply {
                    request: request.clone(),
                    response: Arc::new(tokio::sync::Mutex::new(None)),
                });
                slots.insert(command_id.clone(), slot.clone());
                (slot, true)
            }
        }
    };
    if slot.request != request {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("command_id_conflict"),
        )
        .await;
        return;
    }

    let mut cached = slot.response.lock().await;
    if !handshake.access_current() {
        return;
    }
    if let Some(payload) = cached.as_ref() {
        publish_reply_payload(client, &msg, payload.clone()).await;
        return;
    }

    let runtime = handshake.runtime.clone();
    let reply_ttl = handshake.reply_ttl;
    let capture = Arc::new(Mutex::new(None));
    let gzip_replies = handshake.gzip_replies.load(Ordering::Acquire);
    REPLY_CAPTURE
        .scope(
            capture.clone(),
            REPLY_GZIP.scope(gzip_replies, handle_command(client, msg, handshake)),
        )
        .await;
    *cached = capture.lock().unwrap().clone();
    if inserted {
        let slots = reply_slots.clone();
        let id = command_id.clone();
        let expected = slot.clone();
        runtime.spawn(Box::pin(async move {
            tokio::time::sleep(reply_ttl).await;
            let mut slots = slots.0.lock().unwrap();
            if slots
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(current, &expected))
            {
                slots.remove(&id);
            }
        }));
    }
}

// Business admission requires the current access generation, a committed
// authenticated channel and the controlling device. Broker ACLs alone never
// authorize business. The host retains session/approval ownership, sandbox and
// local business checks when it executes an admitted command.
async fn handle_command(
    client: &async_nats::Client,
    msg: async_nats::Message,
    handshake: HandshakeState,
) {
    // Every handler below reads `remote_host::lean::enabled()` to decide whether
    // to trim a reply to the lean feed. That is this command's connection's
    // declaration — not the machine's, which now has several.
    let lean = handshake.lean_events.load(Ordering::Acquire);
    crate::lean::in_request(lean, handle_command_inner(client, msg, handshake)).await;
}

async fn handle_command_inner(
    client: &async_nats::Client,
    msg: async_nats::Message,
    handshake: HandshakeState,
) {
    if !handshake.access_current() {
        return;
    }
    let cmd: IncomingCmd = match serde_json::from_slice(&msg.payload) {
        Ok(cmd) => cmd,
        Err(e) => {
            reply(
                client,
                &msg,
                false,
                Value::Null,
                Some(&format!("Failed to parse command JSON: {e}")),
            )
            .await;
            return;
        }
    };

    let handshake_command = matches!(
        cmd.cmd_type.as_str(),
        "pair_handshake" | "pair_handshake_confirm"
    );
    if !handshake_command && !handshake.active.load(Ordering::Acquire) {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("pairing_handshake_required"),
        )
        .await;
        return;
    }

    if !handshake_command
        && !cmd.bridge_instance_id.is_empty()
        && cmd.bridge_instance_id != handshake.bridge_instance_id
    {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("remote_access_changed"),
        )
        .await;
        return;
    }

    // Every authenticated command is also a proof the client is there. Recorded
    // before the control gate: a standby device is one this machine asked to sit
    // still, not one it pretends has gone.
    if let Some(link) = handshake.runtime.client_link(&handshake.creds.pair_id) {
        link.observe();
    }

    // Control arbitration happens here, on the machine the two devices are
    // fighting over — not on the platform, which has no downlink and could only
    // be asked about the decision after the fact. A standby device's business
    // commands stop before they reach any handler, which is what makes the
    // moment of takeover clean: nothing already in flight can still execute.
    //
    // The exceptions are the commands a standby device legitimately speaks:
    // asking to take over, reading its own state, unpairing itself, proving it
    // is still there (a standby device that went silent would read as gone), and
    // the pairing handshakes that establish who it even is.
    if !handshake_command
        && !matches!(
            cmd.cmd_type.as_str(),
            "takeover" | "get_presence" | "unpair" | "client_presence" | "client_disconnect"
        )
        && !handshake.runtime.in_control(&handshake.creds.pair_id)
    {
        reply(client, &msg, false, Value::Null, Some("superseded")).await;
        return;
    }
    match cmd.cmd_type.as_str() {
        "pair_handshake" => {
            handle_pair_handshake(client, &msg, &cmd, &handshake).await;
        }
        "pair_handshake_confirm" => {
            handle_pair_handshake_confirm(client, &msg, &cmd, &handshake).await;
        }
        // A device asking for the machine back.
        //
        // This is the only path by which a *connected* device takes control
        // from another, and it exists because the alternative — reclaiming on
        // reconnect — makes two devices evict each other forever. The previous
        // controller is told `standby` on its own channel below, and its client
        // stops reconnecting until its user asks again.
        "takeover" => {
            let moved = handshake
                .runtime
                .claim_control_soon(&handshake.creds.pair_id, handshake.access_epoch);
            if !handshake.access_current() {
                return;
            }
            reply(
                client,
                &msg,
                true,
                json!({
                    "control": "active",
                    // Whether this call changed anything. A device that was
                    // already in control gets `false` and must not read it as
                    // a refusal.
                    "changed": moved.is_some(),
                    "previous": moved,
                }),
                None,
            )
            .await;
            if let Some(previous) = moved {
                handshake.runtime.publish_control(&previous, "standby");
            }
        }
        // Presence is normally pushed on every heartbeat. A client that
        // subscribes after the latest heartbeat would otherwise look offline
        // until the next tick because core NATS subscriptions do not replay old
        // messages.
        "get_presence" => {
            let pair_id = msg
                .subject
                .strip_prefix("p.")
                .and_then(|subject| subject.split('.').next())
                .unwrap_or_default();
            // A standby device gets its own liveness and nothing else. The
            // catalogue is this machine's conversations, and handing the list
            // to every paired device would make "only one controls" a
            // statement about commands alone.
            let payload = if handshake.runtime.in_control(pair_id) {
                handshake
                    .runtime
                    .presence(pair_id, &handshake.bridge_instance_id)
            } else {
                handshake
                    .runtime
                    .light_presence(pair_id, &handshake.bridge_instance_id)
            };
            reply(client, &msg, true, payload, None).await;
        }
        // A beat: the client is still there. Answering is not required (the
        // client sends these without a reply subject), but a beat that does ask
        // gets an answer, which is what lets a test drive it as a request.
        //
        // The proof itself was recorded before the control gate, so a standby
        // device keeps proving it is present without being allowed to command
        // anything — presence and authority are different questions.
        "client_presence" => {
            reply(client, &msg, true, json!({}), None).await;
        }
        // The client is leaving.
        //
        // `reason` decides what the machine does with it. A user's own stop
        // (`disconnected`) gives up the machine: the pointer it held is
        // released and a device that is actually there may take it. An app that
        // went off screen (`background`) does not — a suspended phone is not a
        // user who quit, and taking its machine away every time it is
        // backgrounded would be a different product decision — so it only stops
        // being reported as present, exactly as a lapsed beat would.
        //
        // The departure is applied *before* the acknowledgement. Publishing the
        // reply first would let whoever reads it — the leaving client, another
        // device asking this machine for its presence, the test that drives the
        // goodbye — still see the device present: the reply is flushed onto the
        // wire while the state change is still waiting for its turn on this
        // task. Acknowledging last makes "the reply arrived" mean "the machine
        // has already stopped counting it here", which is the only reading a
        // caller can act on. Nothing here awaits (a promotion publishes from a
        // spawned task), so the departing client is not made to wait on the
        // network for it: a reply nobody reads still costs nothing.
        "client_disconnect" => {
            if cmd.reason == "background" {
                if let Some(link) = handshake.runtime.client_link(&handshake.creds.pair_id) {
                    link.depart();
                }
            } else {
                handshake.runtime.client_left(&handshake.creds.pair_id);
            }
            reply(client, &msg, true, json!({}), None).await;
        }
        // A phone-initiated unpair first reaches the desktop over the existing
        // authenticated command channel. Acknowledge before stopping the
        // bridge, then perform the destructive work in a separate task so the
        // requester can clear itself without waiting for token expiry.
        //
        // It unpairs *this* device only: with several devices sharing a machine,
        // one of them asking to leave is not a request to unpair the others.
        "unpair" => {
            let pair_id = handshake.creds.pair_id.clone();
            reply(client, &msg, true, json!({}), None).await;
            let runtime = handshake.runtime.clone();
            runtime.spawn(Box::pin(async move {
                if !handshake.access_current() {
                    return;
                }
                if let Err(error) = handshake.runtime.unpair_slot(&pair_id).await {
                    eprintln!("remote: phone-initiated unpair failed: {error}");
                }
            }));
        }
        _ => {
            handshake
                .host
                .execute(cmd, &NatsReply { client, msg: &msg })
                .await;
        }
    }
}

use future_remote_protocol::handshake::{
    handshake_transcript, HandshakeTranscript, HANDSHAKE_PROTOCOL_VERSION,
};

async fn handle_pair_handshake(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    cmd: &IncomingCmd,
    state: &HandshakeState,
) {
    // The desktop key pair is derived once: a bad seed fails here, and a
    // successfully parsed pair always signs, so no later fallible step.
    let key_pair = match nkeys::KeyPair::from_seed(&state.creds.nkey_seed) {
        Ok(key_pair) => key_pair,
        Err(error) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
    };
    let desktop_public_key = key_pair.public_key();
    // Validate BEFORE deactivating commands: a garbage handshake must not lock
    // the bridge (active=false gates every command) — only a well-formed
    // handshake from a party that holds the pair's identity may suspend the
    // current session while it re-authenticates.
    //
    // Each field is checked by name so a refusal says *which* one disagreed. A
    // client holding a QR for an identity this bridge no longer serves fails
    // the two `expected_desktop_*` checks, and that is the only place either
    // side can see it: the client just reports that the desktop would not
    // authenticate, and the desktop used to report nothing at all.
    let invalid: Option<&str> = if cmd.protocol_version != HANDSHAKE_PROTOCOL_VERSION {
        Some("protocol_version")
    } else if cmd.pair_id != state.creds.pair_id {
        Some("pair_id")
    } else if cmd.expected_desktop_id != state.creds.desktop_id {
        Some("desktop_id")
    } else if cmd.expected_desktop_public_key != desktop_public_key {
        Some("desktop_public_key")
    } else if !cmd.device_id.starts_with("dev_") {
        Some("device_id")
    } else if !cmd.client_public_key.starts_with('U') {
        Some("client_public_key")
    } else if !(16..=256).contains(&cmd.client_nonce.len()) {
        Some("client_nonce")
    } else {
        None
    };
    if let Some(field) = invalid {
        if let Some(line) = HANDSHAKE_EPISODE.record(
            "pairing_identity_mismatch",
            format!("{field} does not match this bridge's pairing"),
        ) {
            eprintln!("{line}");
        }
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_identity_mismatch"),
        )
        .await;
        return;
    }

    // A candidate challenge must not revoke the already authenticated peer.

    let desktop_nonce = nkeys::KeyPair::new_user().public_key();
    let transcript = handshake_transcript(&HandshakeTranscript {
        pair_id: &state.creds.pair_id,
        desktop_id: &state.creds.desktop_id,
        desktop_public_key: &desktop_public_key,
        bridge_instance_id: &state.bridge_instance_id,
        device_id: &cmd.device_id,
        client_public_key: &cmd.client_public_key,
        client_nonce: &cmd.client_nonce,
        desktop_nonce: &desktop_nonce,
    });
    let signature = URL_SAFE_NO_PAD.encode(
        key_pair
            .sign(transcript.as_bytes())
            .expect("a seed-derived KeyPair always signs"),
    );
    {
        let mut pending = state.pending.lock().unwrap();
        pending.retain(|_, value| value.created_at.elapsed() < Duration::from_secs(30));
        if pending.len() >= 32 {
            return;
        }
        pending.insert(
            desktop_nonce.clone(),
            PendingHandshake {
                transcript,
                created_at: std::time::Instant::now(),
                device_id: cmd.device_id.clone(),
                client_public_key: cmd.client_public_key.clone(),
            },
        );
    }
    reply(
        client,
        msg,
        true,
        json!({
            "protocolVersion": HANDSHAKE_PROTOCOL_VERSION,
            "pairId": state.creds.pair_id,
            "desktopId": state.creds.desktop_id,
            "desktopPublicKey": desktop_public_key,
            "bridgeInstanceId": state.bridge_instance_id,
            "deviceId": cmd.device_id,
            "clientPublicKey": cmd.client_public_key,
            "clientNonce": cmd.client_nonce,
            "desktopNonce": desktop_nonce,
            "desktopSignature": signature,
        }),
        None,
    )
    .await;
}

async fn handle_pair_handshake_confirm(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    cmd: &IncomingCmd,
    state: &HandshakeState,
) {
    let pending = state.pending.lock().unwrap().remove(&cmd.desktop_nonce);
    let Some(pending) = pending else {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_challenge_expired"),
        )
        .await;
        return;
    };
    if !state.access_current()
        || pending.created_at.elapsed() >= Duration::from_secs(30)
        || cmd.device_id != pending.device_id
    {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_identity_mismatch"),
        )
        .await;
        return;
    }
    let signature = URL_SAFE_NO_PAD.decode(&cmd.client_signature).ok();
    let verified = signature
        .and_then(|signature| {
            nkeys::KeyPair::from_public_key(&pending.client_public_key)
                .ok()
                .map(|key| {
                    key.verify(pending.transcript.as_bytes(), &signature)
                        .is_ok()
                })
        })
        .unwrap_or(false);
    if !verified {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_signature_invalid"),
        )
        .await;
        return;
    }
    // Whether this is the pairing's *first* confirmation. Only that one takes
    // control: a client that replays its pairing handshake on a reconnect is
    // reconnecting, not asking for the machine — and if a replay could claim,
    // the two devices would evict each other forever.
    let first_confirmation = !state.confirmed.load(Ordering::Acquire);
    let confirm = || -> Result<(), crate::Error> {
        if !state.confirmed.load(Ordering::Acquire) {
            state.runtime.save_binding(&state.creds)?;
            state.confirmed.store(true, Ordering::Release);
        }
        state.active.store(true, Ordering::Release);
        Ok(())
    };
    let committed = match state.access_epoch {
        Some(epoch) => state.runtime.access().commit(epoch, confirm),
        None => Some(confirm()),
    };
    match committed {
        None => return,
        Some(Err(error)) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
        Some(Ok(())) => {}
    }
    // Pairing this device is *not* the act that puts it in front of the machine
    // when another device already holds it. The user is the one who decides to
    // displace a controller, and the client they are looking at is where that
    // decision is made: this device comes up in standby, its presence names the
    // current holder, and its own "take over" is what moves the machine. Making
    // the mere handshake claim would silently disconnect whatever the other
    // person was in the middle of.
    reply(
        client,
        msg,
        true,
        json!({
            "confirmed": true,
            "pairId": state.creds.pair_id,
            "desktopId": state.creds.desktop_id,
            "bridgeInstanceId": state.bridge_instance_id,
            "deviceId": cmd.device_id,
            "desktopNonce": cmd.desktop_nonce,
            "features": ["file_transfer_v1", "file_download_v2", "approval_tier_v1", "continue_run_v1", "prompt_receipt_v1", "session_files_v1", "skills_v1", "selective_events_v1", "workspace_pinning_v1", "workspace_create_v1", "desktop_settings_v1", "skill_management_v1", "compaction_v1", "provider_management_v1", "fork_v1", "tasks_v1", "task_deps_v1"],
            // A device that is not in control gets its own liveness and nothing
            // else: the conversation list belongs to the machine, and this reply
            // is also what a replayed handshake receives. `control` and `holder`
            // are how it learns whether it may drive at all, and whom it would
            // displace.
            "presence": if state.runtime.in_control(&state.creds.pair_id) {
                state.runtime.presence(&state.creds.pair_id, &state.bridge_instance_id)
            } else {
                state.runtime.light_presence(&state.creds.pair_id, &state.bridge_instance_id)
            },
        }),
        None,
    )
    .await;
    // After the answer, for the same reason as the secure handshake: a claim
    // writes to disk, and no reply may wait on that. And like the secure path,
    // only a machine nobody holds is claimed by connecting to it.
    if first_confirmation
        && state.access_current()
        && !state.runtime.held_by_another(&state.creds.pair_id)
    {
        let moved = state
            .runtime
            .claim_control_soon(&state.creds.pair_id, state.access_epoch);
        if !state.access_current() {
            return;
        }
        if let Some(previous) = moved {
            state.runtime.publish_control(&previous, "standby");
        }
    }
}

/// Send a unified request-reply response (in `RpcResponse` shape), and flush to ensure timely delivery.
async fn reply(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    success: bool,
    data: Value,
    error: Option<&str>,
) {
    if msg.reply.is_none() {
        return;
    }
    let body = json!({
        "type": "response",
        "success": success,
        "data": data,
        "error": error,
    });
    // Plain JSON is the default. When FUTURE_REMOTE_JSON_GZIP explicitly opts
    // in, only large replies that actually shrink are gzip-compressed; mobile
    // recognizes standard gzip magic bytes without a negotiation round.
    let payload = encode_reply_payload(&body);
    let _ = REPLY_CAPTURE.try_with(|capture| {
        *capture.lock().unwrap() = Some(payload.clone());
    });
    publish_reply_payload(client, msg, payload).await;
}

fn encode_reply_payload(body: &Value) -> Vec<u8> {
    encode_reply_payload_with_gzip(body, remote_json_gzip_enabled())
}

use crate::reply::encode_reply_payload_with_gzip;

async fn publish_reply_payload(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    payload: Vec<u8>,
) {
    let Some(reply_subject) = msg.reply.clone() else {
        return;
    };
    let payload = match crate::secure::REPLY.try_with(|security| security.seal(&payload)) {
        Ok(Ok(wire)) => wire,
        Ok(Err(_)) => return,
        Err(_) => {
            #[cfg(not(any(test, feature = "test-support")))]
            return;
            #[cfg(any(test, feature = "test-support"))]
            {
                payload
            }
        }
    };
    if let Err(error) = client.publish(reply_subject, payload.into()).await {
        eprintln!("FutureOS: failed to publish remote command reply: {error}");
        return;
    }
    if let Err(error) = client.flush().await {
        eprintln!("FutureOS: failed to flush remote command reply: {error}");
    }
}

struct NatsReply<'a> {
    client: &'a async_nats::Client,
    msg: &'a async_nats::Message,
}
impl ReplySink for NatsReply<'_> {
    fn send<'a>(
        &'a self,
        success: bool,
        data: Value,
        error: Option<String>,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move { reply(self.client, self.msg, success, data, error.as_deref()).await })
    }
}

tokio::task_local! { static COMMAND_RUNTIME: Arc<dyn CommandRuntime>; }

/// Inputs to one pairing's command endpoint. Runtime state is migrating into
/// Server; this interface carries the existing context without product imports.
pub(crate) struct CommandServices {
    pub runtime: Arc<dyn CommandRuntime>,
    pub business: Arc<dyn BusinessHost>,
    pub reply_ttl: Duration,
}
pub(crate) trait CommandRuntime: crate::secure::HandshakeHost {
    fn access(&self) -> &crate::lifecycle::AccessEpoch;
    fn client_identity(
        &self,
        pair: &str,
    ) -> Option<Arc<Mutex<Option<crate::types::ClientIdentity>>>>;
    /// The pairing's record of whether its client is at the other end.
    fn client_link(&self, pair: &str) -> Option<Arc<crate::supervisor::ClientLink>>;
    /// The client on `pair` said it is leaving.
    fn client_left(&self, pair: &str);
    fn coalesce_events(&self, pair: &str) -> Arc<AtomicBool>;
    fn catalog_activity(&self, pair: &str) -> Arc<AtomicBool>;
    fn lean_events(&self, pair: &str) -> Arc<AtomicBool>;
    fn in_control(&self, pair: &str) -> bool;
    /// Nobody holds the machine and `pair`'s client is the only one here — the
    /// one case a reconnect may take control (see `Supervisor::alone_and_unowned`).
    fn alone_and_unowned(&self, pair: &str) -> bool;
    fn held_by_another(&self, pair: &str) -> bool;
    fn claim_control_soon(&self, pair: &str, epoch: Option<u64>) -> Option<String>;
    fn light_presence(&self, pair: &str, bridge: &str) -> Value;
    fn publish_control(&self, pair: &str, control: &str);
    fn unpair_slot<'a>(
        &'a self,
        pair: &'a str,
    ) -> futures::future::BoxFuture<'a, Result<(), crate::Error>>;
    fn gzip_override(&self) -> bool;
    fn spawn(&self, future: futures::future::BoxFuture<'static, ()>);
}
impl HandshakeState {
    pub fn credentials(&self) -> &crate::PairingCreds {
        &self.creds
    }
}
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn a_declared_client_is_normalised() {
        assert_eq!(
            declared_identity(&IncomingCmd {
                device_kind: "  desktop  ".into(),
                device_name: "  FutureOS Desktop (studio-imac)  ".into(),
                ..Default::default()
            }),
            Some(crate::types::ClientIdentity {
                name: "FutureOS Desktop (studio-imac)".into(),
                kind: "desktop".into(),
            })
        );
    }
    #[test]
    fn a_blank_name_means_did_not_say() {
        assert_eq!(
            declared_identity(&IncomingCmd {
                device_name: "   ".into(),
                device_kind: "mobile".into(),
                ..Default::default()
            }),
            None
        );
        assert_eq!(declared_identity(&IncomingCmd::default()), None);
    }
    #[test]
    fn an_unknown_kind_is_kept_verbatim() {
        assert_eq!(
            declared_identity(&IncomingCmd {
                device_name: "Watch".into(),
                device_kind: "wearable".into(),
                ..Default::default()
            })
            .map(|client| client.kind),
            Some("wearable".into())
        );
        // And an absent kind is an empty string, not a missing one: the UI reads
        // a kind it does not know as "a connected device".
        assert_eq!(
            declared_identity(&IncomingCmd {
                device_name: "Something".into(),
                ..Default::default()
            })
            .map(|client| client.kind),
            Some(String::new())
        );
    }
}

#[cfg(test)]
mod context_tests;
