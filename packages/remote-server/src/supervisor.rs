//! Runtime state and control arbitration owned by one Server supervisor.
//! Production state is encapsulated; test-support exposes fixture installation.
use crate::{
    commands,
    diagnostics::DropCounters,
    events::EventPublish,
    health::NatsHealth,
    lifecycle, secure,
    services::{Invitation, RemoteHost},
    tasks::{self, NativeHandle},
    types::ClientIdentity,
    PairingCreds,
};
use futures::future::BoxFuture;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};

pub use crate::tasks::TaskHandle;
mod publisher;
mod status;
pub use publisher::PublisherTiming;
pub use status::StatusProjection;

mod context;
mod link;
mod runtime;
mod shutdown;
mod start;
mod transport;
pub use link::{ClientLink, CLIENT_PRESENCE_BEAT, CLIENT_PRESENCE_FEATURE, CLIENT_PRESENCE_LEASE};
pub use runtime::{RuntimeOptions, TaskExecutor, ValidationSurface};
pub use shutdown::{
    abort_generation, power_transition, send_mobile_disconnect_notice, PowerEvent, PowerTransition,
};
pub use start::SlotPlan;
pub use transport::TransportTasks;

pub use crate::connection::ConnectedNats;

/// Active remote connection. Holds async-nats client + command/event tasks;
/// on stop, aborts the tasks and drops the client.
pub struct RemoteState {
    pub security: secure::Transport,
    pub generation_id: u64,
    /// Raw client, kept to derive real connection state for [`status`].
    pub client: async_nats::Client,
    pub nats_health: Arc<NatsHealth>,
    pub nats_url: String,
    pub pair_id: String,
    pub desktop_id: String,
    pub desktop_public_key: String,
    pub bridge_instance_id: String,
    /// Ordered event queue → single drain task per connection. The drain holds
    /// a clone of the client so the connection stays alive while events are in
    /// flight.
    pub event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    pub drop_counters: Arc<DropCounters>,
    pub event_task: TaskHandle,
    pub cmd_task: TaskHandle,
    pub transfer_task: TaskHandle,
    pub heartbeat_task: TaskHandle,
    pub refresh_task: TaskHandle,
    /// `None` outside the test environment, or when the optional test web
    /// server failed to bind. The phone bridge remains available either way.
    pub web_task: Option<TaskHandle>,
    /// Test-only web client URL for THIS machine; `None` outside the test
    /// environment or when bind failed.
    pub web_url: Option<String>,
    /// Test-only web client URL a phone on the same LAN can reach; `None`
    /// outside the test environment, when bind failed, or without a LAN route.
    pub web_lan_url: Option<String>,
    /// The one-shot pairing code issued at start, kept (with its expiry) so the
    /// UI can re-show it after navigation until it expires — no longer a
    /// fire-once value lost the moment you switch views.
    pub pairing_code: Option<String>,
    pub pairing_code_expires_at: Option<i64>,
    /// New pairings remain pending until the client and bridge complete the
    /// signed application-level handshake.
    pub pairing_confirmed: Arc<AtomicBool>,
}

macro_rules! define_supervisor {
    ($visibility:vis) => {
        /// One instance owns remote intent, access epochs, connections, recovery
        /// budgets and scheduled work. Applications consume its status projection.
        pub struct Supervisor {
            $visibility access: Arc<lifecycle::AccessEpoch>,
            /// One runtime per paired device, keyed by `pair_id`.
            ///
            /// Every slot keeps its own NATS connection, presence, credential refresh
            /// and command loop, so a device losing its link — or losing its JWT — is
            /// that device's problem and never the other one's. What the slots do *not*
            /// share is the right to drive this machine; see [`Supervisor::active`].
            $visibility state: Mutex<HashMap<String, RemoteState>>,
            $visibility bridge_shared: Mutex<HashMap<String, BridgeRuntimeShared>>,
            /// The one device allowed to drive this machine.
            ///
            /// Mirrored from the pairing book, and only ever moved by an explicit claim
            /// — a confirmed pairing handshake or a `takeover` request. It is
            /// deliberately *not* moved by a reconnect: a device that was superseded
            /// must come back as a standby, or the two devices would take turns
            /// evicting each other for as long as either one keeps reconnecting.
            $visibility active: Arc<Mutex<Option<String>>>,
            /// The devices this process has a credential for.
            ///
            /// Kept in memory so [`Supervisor::in_control`] — which every command and
            /// every presence packet reads — never touches the filesystem. It is
            /// written by start (the slots it served), by a slot being added, and by an
            /// unpair.
            $visibility known: Mutex<std::collections::HashSet<String>>,
            $visibility last_error_code: Mutex<Option<String>>,
            $visibility start_lock: tokio::sync::Mutex<()>,
            $visibility start_requested: AtomicBool,
            $visibility invite_when_empty: AtomicBool,
            $visibility suspended: AtomicBool,
            $visibility resume_recovery_running: AtomicBool,
            $visibility start_retry_running: AtomicBool,
            $visibility start_retry_attempts: AtomicU64,
            $visibility start_retry_since: AtomicU64,
            $visibility start_retry_next_at: AtomicU64,
            $visibility credential_refreshing: AtomicBool,
            $visibility runtime_reconnect_attempts: AtomicU8,
            $visibility runtime_failure_window_started: AtomicU64,
            $visibility web_reconnect_running: AtomicBool,
            $visibility web_reconnect_attempts: AtomicU8,
            $visibility web_owner: Mutex<Option<String>>,
            tasks: Mutex<Vec<TaskHandle>>,
            retiring: Mutex<Vec<TaskHandle>>,
            generation_tasks: Mutex<Vec<TaskHandle>>,
            native_tasks: Mutex<Vec<NativeHandle>>,
            control: Arc<ControlPersistence>,
            host: Arc<dyn RemoteHost>,
            executor: Arc<dyn TaskExecutor>,
            publisher_failures: crate::diagnostics::FailureEpisode,
            start_failures: Arc<crate::diagnostics::FailureEpisode>,
            credential_failures: crate::diagnostics::FailureEpisode,
            maintenance: Mutex<Option<TaskHandle>>,
            closed: AtomicBool,
            #[cfg(any(test, feature = "test-support"))]
            $visibility hooks: runtime::RuntimeHooks,
        }
    };
}
#[cfg(any(test, feature = "test-support"))]
define_supervisor!(pub);
#[cfg(not(any(test, feature = "test-support")))]
define_supervisor!(pub(crate));

#[derive(Default)]
struct ControlPersistence {
    write_lock: Mutex<()>,
    revision: AtomicU64,
}

impl Supervisor {
    /// Create one explicitly shared runtime handle; scheduled futures capture
    /// this instance rather than looking up an application singleton.
    pub fn new(host: Arc<dyn RemoteHost>, executor: Arc<dyn TaskExecutor>) -> Arc<Self> {
        Arc::new(Self {
            access: Arc::new(lifecycle::AccessEpoch::new()),
            state: Mutex::new(HashMap::new()),
            bridge_shared: Mutex::new(HashMap::new()),
            active: Arc::new(Mutex::new(None)),
            known: Mutex::new(std::collections::HashSet::new()),
            last_error_code: Mutex::new(None),
            start_lock: tokio::sync::Mutex::const_new(()),
            start_requested: AtomicBool::new(false),
            invite_when_empty: AtomicBool::new(true),
            suspended: AtomicBool::new(false),
            resume_recovery_running: AtomicBool::new(false),
            start_retry_running: AtomicBool::new(false),
            start_retry_attempts: AtomicU64::new(0),
            start_retry_since: AtomicU64::new(0),
            start_retry_next_at: AtomicU64::new(0),
            credential_refreshing: AtomicBool::new(false),
            runtime_reconnect_attempts: AtomicU8::new(0),
            runtime_failure_window_started: AtomicU64::new(0),
            web_reconnect_running: AtomicBool::new(false),
            web_reconnect_attempts: AtomicU8::new(0),
            web_owner: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
            retiring: Mutex::new(Vec::new()),
            generation_tasks: Mutex::new(Vec::new()),
            native_tasks: Mutex::new(Vec::new()),
            control: Arc::new(ControlPersistence::default()),
            host,
            executor,
            publisher_failures: Default::default(),
            start_failures: Default::default(),
            credential_failures: Default::default(),
            maintenance: Mutex::new(None),
            closed: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            hooks: Default::default(),
        })
    }
}

impl Supervisor {
    pub fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut tasks = self.tasks.lock().unwrap();
        if self.closed.load(Ordering::Acquire)
            || !self.start_requested.load(Ordering::Acquire)
            || self.suspended.load(Ordering::Acquire)
        {
            return;
        }
        tasks.retain(|task| !task.is_finished());
        let (handle, future) = tasks::tracked(Box::pin(future));
        tasks.push(handle);
        self.executor.spawn(future);
    }
    /// After cancellation, wait for future destruction and real native completion
    /// within one total budget, including retired transport generations.
    pub async fn wait_tasks(&self, budget: std::time::Duration) -> Result<(), crate::Error> {
        let mut handles = std::mem::take(&mut *self.retiring.lock().unwrap());
        let generation_start = handles.len();
        handles.extend(std::mem::take(&mut *self.generation_tasks.lock().unwrap()));
        let native = std::mem::take(&mut *self.native_tasks.lock().unwrap());
        let result = tokio::time::timeout(budget, async {
            for task in &handles {
                task.clone().completed().await;
            }
            for task in &native {
                task.clone().completed().await;
            }
        })
        .await;
        if result.is_err() {
            self.generation_tasks.lock().unwrap().extend(
                handles
                    .split_off(generation_start)
                    .into_iter()
                    .filter(|task| !task.is_finished()),
            );
            self.retiring
                .lock()
                .unwrap()
                .extend(handles.into_iter().filter(|task| !task.is_finished()));
            self.native_tasks
                .lock()
                .unwrap()
                .extend(native.into_iter().filter(|task| !task.is_finished()));
            return Err(crate::Error::Message(
                "remote_server_cleanup_timeout".into(),
            ));
        }
        Ok(())
    }
    /// What the client on `pair_id` said it was.
    ///
    /// An unknown pairing gets an empty slot rather than the previous client's
    /// description: a device that connected before must not be described on a
    /// pairing it is not part of.
    pub fn client_identity(&self, pair_id: &str) -> Option<Arc<Mutex<Option<ClientIdentity>>>> {
        self.bridge_shared
            .lock()
            .unwrap()
            .get(pair_id)
            .map(|shared| shared.client_identity.clone())
    }

    /// Whether the client on `pair_id` is still there, and the record that
    /// answers it.
    ///
    /// An unknown pairing gets a fresh link rather than reporting somebody
    /// else's client as present: presence belongs to the pairing it was proved
    /// on, exactly like the identity above.
    pub fn client_link(&self, pair_id: &str) -> Arc<link::ClientLink> {
        self.shared_or_insert(pair_id).client_link
    }

    /// The live-lane capabilities declared for `pair_id`. Always off for a
    /// connection that has not declared them: capability is never inherited
    /// from whoever connected before.
    pub fn coalesce_events(&self, pair_id: &str) -> Arc<AtomicBool> {
        self.shared_or_insert(pair_id).coalesce_events
    }

    /// Whether the client on `pair_id` asked for the catalog's recency packets
    /// instead of a whole session list per second of streaming. Same per-pairing
    /// rule as [`Supervisor::coalesce_events`].
    pub fn catalog_activity(&self, pair_id: &str) -> Arc<AtomicBool> {
        self.shared_or_insert(pair_id).catalog_activity
    }

    /// Whether the connection on `pair_id` declared the lean feed.
    pub fn lean_events(&self, pair_id: &str) -> Arc<AtomicBool> {
        self.shared_or_insert(pair_id).lean_events
    }

    /// The capability state of `pair_id`, creating an empty one if this pairing
    /// has no runtime entry yet.
    ///
    /// A capability is recorded on a declaration and read on every request, and
    /// a declaration can arrive before the runtime is installed (the command
    /// loop subscribes first). Answering with a throwaway flag there would look
    /// like a declaration that silently did nothing.
    fn shared_or_insert(&self, pair_id: &str) -> BridgeRuntimeShared {
        let mut guard = self.bridge_shared.lock().unwrap();
        guard
            .entry(pair_id.to_string())
            .or_insert_with(|| BridgeRuntimeShared {
                reply_slots: commands::new_reply_slots(),
                pairing_confirmed: Arc::new(AtomicBool::new(false)),
                bridge_instance_id: format!("bridge_{}", nkeys::KeyPair::new_user().public_key()),
                drop_counters: Arc::new(DropCounters::new()),
                coalesce_events: Arc::new(AtomicBool::new(false)),
                catalog_activity: Arc::new(AtomicBool::new(false)),
                lean_events: Arc::new(AtomicBool::new(false)),
                client_identity: Arc::new(Mutex::new(None)),
                client_link: Arc::new(link::ClientLink::default()),
                next_generation_id: Arc::new(AtomicU64::new(1)),
                handshake: Arc::new(Mutex::new(None)),
                invitation: None,
            })
            .clone()
    }

    /// Whether `pair_id` is the device driving this machine.
    ///
    /// Read on every command and every published packet, so it is a plain lock
    /// acquisition over a short critical section rather than anything clever.
    ///
    /// "Nothing claimed" answers yes: that is a machine with no paired device
    /// (so nothing to arbitrate) or a fresh process before the pointer is read
    /// from disk. A pointer that names a device this process does not have is
    /// treated the same way — see [`Supervisor::device_is_known`].
    pub fn in_control(&self, pair_id: &str) -> bool {
        match self.active_pair() {
            Some(active) if active == pair_id => true,
            Some(active) => {
                // A pointer naming a device this machine neither runs nor has
                // a credential for is stale — it cannot be in front of the
                // machine. Honouring it would lock every real device out of
                // its own desktop.
                !self.device_is_known(&active)
            }
            // Nothing has claimed the machine. On a machine with no paired
            // device that is the whole truth and there is nothing to protect;
            // on one that *does* have devices it means nobody was given the
            // machine, and nobody may act as if they were.
            None => self.known.lock().unwrap().is_empty(),
        }
    }

    /// Whether `pair_id` is a device this process actually has.
    fn device_is_known(&self, pair_id: &str) -> bool {
        self.state.lock().unwrap().contains_key(pair_id)
            || self.known.lock().unwrap().contains(pair_id)
    }

    /// Whether a *different* device currently holds the machine.
    ///
    /// This is the one question the handshake asks before it moves anything:
    /// connecting must not be the same act as taking over. A pointer naming a
    /// device this machine does not have is stale and does not count — it
    /// cannot be in front of the machine.
    pub fn held_by_another(&self, pair_id: &str) -> bool {
        match self.active_pair() {
            Some(active) if active != pair_id => self.device_is_known(&active),
            _ => false,
        }
    }

    /// What the device in control says it is, for a client that has to be told
    /// who it would displace.
    ///
    /// The name comes from the controller's own `secure_ready`; a machine whose
    /// controller never declared one has nothing to show, and the client falls
    /// back to a neutral wording rather than inventing an identity.
    pub fn controller_identity(&self) -> Option<ClientIdentity> {
        let active = self.active_pair()?;
        let slot = self.client_identity(&active)?;
        let identity = slot.lock().unwrap().clone();
        identity
    }

    /// Forget every device and the controller — a machine-wide stop.
    pub fn reset_devices(&self) {
        self.control.revision.fetch_add(1, Ordering::AcqRel);
        self.known.lock().unwrap().clear();
        *self.active.lock().unwrap() = None;
    }

    pub fn remember_device(&self, pair_id: &str) {
        self.known.lock().unwrap().insert(pair_id.to_string());
    }

    pub fn forget_device(&self, pair_id: &str) {
        self.known.lock().unwrap().remove(pair_id);
    }

    /// The device in control, if one claimed it.
    pub fn active_pair(&self) -> Option<String> {
        self.active.lock().unwrap().clone()
    }

    /// The device that actually drives the machine right now.
    ///
    /// Same rule as [`Supervisor::in_control`], in the shape a publisher needs:
    /// the claimed pointer, else the only connected device. `None` when several
    /// are connected and none has been given the machine — in which case there
    /// is nobody to publish to, which is the honest answer.
    pub fn controlling_pair(&self) -> Option<String> {
        if let Some(active) = self.active_pair() {
            return Some(active);
        }
        let states = self.state.lock().unwrap();
        if states.len() == 1 {
            return states.keys().next().cloned();
        }
        None
    }

    /// Adopt the persisted pointer without claiming anything.
    pub fn restore_active(&self, pair_id: Option<String>) {
        self.control.revision.fetch_add(1, Ordering::AcqRel);
        *self.active.lock().unwrap() = pair_id;
    }

    /// Move control to `pair_id`. Returns the device that lost it, if any.
    ///
    /// The persisted pointer is written before returning: a control that a
    /// restart could not reproduce would hand the machine to a different device
    /// after a crash, which is exactly the decision the user made and must not
    /// be lost. Callers on a reply path use [`Supervisor::claim_control_soon`].
    pub fn claim_control(&self, pair_id: &str) -> Option<String> {
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        let epoch = self.access.current();
        let (changed, previous, revision) =
            self.access.commit(epoch, || self.move_control(pair_id))?;
        if changed {
            self.control_writer(pair_id, epoch, revision)();
        }
        previous
    }

    /// Persist off the reply path. The epoch excludes stopped generations, and
    /// revision ordering prevents an older claim overwriting a later takeover.
    pub fn claim_control_soon(&self, pair_id: &str) -> Option<String> {
        self.claim_control_for(pair_id, Some(self.access.current()))
    }
    pub fn claim_control_for(&self, pair_id: &str, expected: Option<u64>) -> Option<String> {
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        let epoch = expected.unwrap_or_else(|| self.access.current());
        self.access
            .commit(epoch, || {
                let (changed, previous, revision) = self.move_control(pair_id);
                if changed {
                    let job = self.control_writer(pair_id, epoch, revision);
                    let (handle, job) = tasks::native(job);
                    let mut native = self.native_tasks.lock().unwrap();
                    native.retain(|task| !task.is_finished());
                    native.push(handle);
                    self.executor.spawn_native(job);
                }
                previous
            })
            .flatten()
    }
    fn control_writer(
        &self,
        pair_id: &str,
        epoch: u64,
        revision: u64,
    ) -> Box<dyn FnOnce() + Send + 'static> {
        let access = self.access.clone();
        let control = self.control.clone();
        let active = self.active.clone();
        let host = self.host.clone();
        let persisted = pair_id.to_string();
        Box::new(move || {
            let result = {
                // Serialize native writes, while admission/control changes never
                // wait for filesystem work. A queued stale writer is discarded;
                // already-running native work is observed by wait_tasks.
                let _write = control.write_lock.lock().unwrap();
                if access.current() != epoch
                    || control.revision.load(Ordering::Acquire) != revision
                    || active.lock().unwrap().as_deref() != Some(persisted.as_str())
                {
                    return;
                }
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    host.set_active(&persisted)
                }))
            };
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    eprintln!("remote: could not persist the controlling device: {error}")
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        })
    }

    /// The in-memory half: `(did it move, who held it before)`.
    ///
    /// A move from "nobody" counts: that is the case a machine with one device
    /// hits at start, and the one that must still be written down.
    fn move_control(&self, pair_id: &str) -> (bool, Option<String>, u64) {
        let mut active = self.active.lock().unwrap();
        if active.as_deref() == Some(pair_id) {
            return (false, None, self.control.revision.load(Ordering::Acquire));
        }
        let revision = self
            .control
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        (true, active.replace(pair_id.to_string()), revision)
    }

    /// Forget the pointer, e.g. when the controlling device is unpaired.
    pub fn release_control(&self, pair_id: &str) {
        let mut active = self.active.lock().unwrap();
        if active.as_deref() == Some(pair_id) {
            self.control.revision.fetch_add(1, Ordering::AcqRel);
            *active = None;
        }
    }

    /// The client on `pair_id` said it is leaving (`client_disconnect`).
    ///
    /// The machine stops treating it as the one in front of it: a pointer it
    /// held is released. If exactly one *other* device is actually present, the
    /// machine goes to it — that device was waiting behind a controller that has
    /// gone, and handing it over is what "the connected one takes over" means now
    /// that the departed one is not connected. Several others is no promotion:
    /// choosing one for them would decide who is sitting in front of the computer,
    /// which is their own call (their row offers "let this control").
    ///
    /// Releasing only touches the in-memory pointer, like
    /// [`Supervisor::disconnect_device`]: the book keeps naming the device, so it
    /// takes the machine back by reconnecting once nobody else holds it.
    pub fn client_left(self: &Arc<Self>, pair_id: &str) {
        self.client_link(pair_id).depart();
        self.release_control(pair_id);
        let present = self.present_devices_except(pair_id);
        if let [only] = present.as_slice() {
            self.claim_control_soon(only);
            self.publish_control(only, "active");
        }
    }

    /// Nobody holds the machine, and `pair_id`'s client is the only one here.
    ///
    /// The one case a *reconnect* may take control: a device that disconnected
    /// (and so gave the machine up, see [`Supervisor::client_left`]) is coming
    /// back as the only client in front of the computer. Without this it would
    /// return as a standby and need its user to ask for the machine it alone is
    /// using — which is the same trap the first-handshake rule avoids for a
    /// fresh pairing.
    ///
    /// Deliberately narrow. A pointer names somebody ⇒ not this; another client
    /// is present ⇒ not this (choosing between them is their own decision, the
    /// rule every other promotion path follows). Two devices reconnecting at
    /// the same instant can both see the machine unowned; one of them wins and
    /// the other's next handshake is a plain reconnect, so the pair cannot
    /// evict each other in a loop.
    pub fn alone_and_unowned(&self, pair_id: &str) -> bool {
        self.active_pair().is_none() && self.present_devices_except(pair_id).is_empty()
    }

    /// The devices whose clients are actually there right now, minus `except`.
    fn present_devices_except(&self, except: &str) -> Vec<String> {
        // `state` first, then `bridge_shared` through `client_link` — the same
        // order the status projection takes them in.
        let states = self.state.lock().unwrap();
        states
            .iter()
            .filter(|(pair_id, state)| {
                pair_id.as_str() != except
                    && status::connected(state)
                    && self.client_link(pair_id).present()
            })
            .map(|(pair_id, _)| pair_id.clone())
            .collect()
    }

    pub fn cancel_tasks(&self) {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        for task in &tasks {
            task.abort();
        }
        let mut retiring = self.retiring.lock().unwrap();
        retiring.retain(|task| !task.is_finished());
        retiring.extend(tasks);
        self.resume_recovery_running.store(false, Ordering::Release);
        self.start_retry_running.store(false, Ordering::Release);
        self.web_reconnect_running.store(false, Ordering::Release);
    }
}

/// State whose correctness spans credential and transport generations. A
/// generation swap must never clear command single-flight replies or pairing
/// confirmation, otherwise a retried command can execute twice and a paired
/// phone can be forced through an unnecessary claim flow.
#[derive(Clone)]
pub struct BridgeRuntimeShared {
    pub reply_slots: commands::ReplySlots,
    pub pairing_confirmed: Arc<AtomicBool>,
    pub coalesce_events: Arc<AtomicBool>,
    /// Whether this client may be sent recency packets instead of a whole
    /// session list whenever a run writes its thread's activity timestamp. Lives
    /// with the other capability flags and for the same reason: a client that
    /// cannot apply a packet must keep receiving whole snapshots.
    pub catalog_activity: Arc<AtomicBool>,
    /// Whether this connection declared the lean feed. Per pairing like the
    /// others: trimming one device's history to fields another device asked to
    /// skip would be a silent narrowing of what it receives.
    pub lean_events: Arc<AtomicBool>,
    /// What the connected client said it was, until a new pairing replaces it.
    ///
    /// Spans generations like the fields above, and for the same reason: a
    /// credential refresh is the *same* device reconnecting, so the UI must not
    /// blink to "unknown device" every time a JWT is renewed. A re-minted
    /// invitation does start a new runtime, which is right — a different device
    /// may claim it.
    pub client_identity: Arc<Mutex<Option<ClientIdentity>>>,
    /// Whether the device's client is still at the other end (see
    /// [`link::ClientLink`]).
    ///
    /// Spans generations for the same reason as the identity above: a credential
    /// refresh reconnects the *same* client, which is here either way, while a
    /// new invitation starts a new runtime — and an empty room until somebody
    /// claims it.
    pub client_link: Arc<link::ClientLink>,
    pub bridge_instance_id: String,
    pub drop_counters: Arc<DropCounters>,
    pub next_generation_id: Arc<AtomicU64>,
    pub handshake: Arc<Mutex<Option<commands::HandshakeState>>>,
    /// The invitation this runtime serves. See [`InvitationKey`].
    invitation: Option<InvitationKey>,
}

/// Identity of the invitation a runtime serves: the two public keys the phone
/// authenticates a pairing against — the desktop NKey the QR names as
/// `desktopKey`, and the secure identity it carries as `secureKey`.
///
/// A credential *refresh* keeps both (same NKey seed, same secure identity), so
/// a refreshed JWT must keep its runtime. A re-minted invitation never does: it
/// generates a fresh NKey pair and a fresh secure identity. Reusing a runtime
/// across that boundary leaves the QR on screen describing keys the bridge
/// cannot prove — the phone scans it and every attempt fails, silently, with no
/// way out but stopping the bridge.
#[derive(Clone, PartialEq, Eq)]
pub struct InvitationKey {
    desktop_public_key: String,
    secure_public_key: String,
}

impl InvitationKey {
    /// `None` when the NKey seed cannot be read at all — the bridge can then
    /// serve no invitation, so a cached runtime must never be reused for it.
    fn of(creds: &PairingCreds) -> Option<Self> {
        Some(Self {
            desktop_public_key: nkeys::KeyPair::from_seed(&creds.nkey_seed)
                .ok()?
                .public_key(),
            secure_public_key: creds
                .secure
                .as_ref()
                .map(|identity| identity.public_key.clone())
                .unwrap_or_default(),
        })
    }
}

impl Supervisor {
    pub fn shared_runtime(
        &self,
        creds: &PairingCreds,
        pairing_confirmed: bool,
        rotate_epoch: bool,
    ) -> BridgeRuntimeShared {
        let pair_id = creds.pair_id.as_str();
        let invitation = InvitationKey::of(creds);
        let mut guard = self.bridge_shared.lock().unwrap();
        // Reuse only while the runtime still serves the very invitation these
        // credentials describe. Single-flight replies and pairing confirmation are
        // meant to survive a credential generation swap; they must not survive a
        // new invitation, whose identity nothing on screen or on the phone agrees
        // with.
        //
        // Keyed by `pair_id`, so two devices never share any of it: their reply
        // single-flight caches, capability flags and client identities are separate
        // by construction rather than by remembering to compare ids.
        let reusable = guard.get(pair_id).is_some_and(|shared| {
            invitation.is_some() && shared.invitation.as_ref() == invitation.as_ref()
        });
        if reusable {
            let shared = guard.get_mut(pair_id).expect("checked above");
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
            reply_slots: commands::new_reply_slots(),
            pairing_confirmed: Arc::new(AtomicBool::new(pairing_confirmed)),
            bridge_instance_id: format!("bridge_{}", nkeys::KeyPair::new_user().public_key()),
            drop_counters: Arc::new(DropCounters::new()),
            coalesce_events: Arc::new(AtomicBool::new(false)),
            catalog_activity: Arc::new(AtomicBool::new(false)),
            lean_events: Arc::new(AtomicBool::new(false)),
            client_identity: Arc::new(Mutex::new(None)),
            client_link: Arc::new(link::ClientLink::default()),
            next_generation_id: Arc::new(AtomicU64::new(1)),
            handshake: Arc::new(Mutex::new(None)),
            invitation,
        };
        guard.insert(pair_id.to_string(), shared.clone());
        shared
    }
}

// Recovery budgets belong to the supervisor and survive transport replacement.
pub const MAX_RUNTIME_RECONNECT_ATTEMPTS: u8 = 3;
pub const RUNTIME_FAILURE_WINDOW_MS: u64 = 10 * 60 * 1_000;
/// Do not forgive a crash-loop merely because a replacement generation stayed
/// alive for one poll. Only a sustained healthy minute resets the budget.
pub const RUNTIME_HEALTHY_RESET_SECS: u8 = 60;
pub const MAX_WEB_RECONNECT_ATTEMPTS: u8 = 3;

/// Single-slot view of the runtime map, for tests.
///
/// Production code addresses one device at a time by `pair_id`, because that is
/// the whole point of the map. The bridge tests predate multi-device and each
/// drives exactly one pairing, so they keep their `Option`-shaped vocabulary
/// through this adapter instead of every assertion learning about the map —
/// which would obscure what they are asserting about a single connection.
#[cfg(any(test, feature = "test-support"))]
pub trait SingleSlotMap {
    fn as_ref(&self) -> Option<&RemoteState>;
    fn as_mut(&mut self) -> Option<&mut RemoteState>;
    fn is_none(&self) -> bool;
    fn is_some(&self) -> bool;
    fn replace(&mut self, state: RemoteState) -> Option<RemoteState>;
    fn take(&mut self) -> Option<RemoteState>;
}

#[cfg(any(test, feature = "test-support"))]
impl SingleSlotMap for HashMap<String, RemoteState> {
    fn as_ref(&self) -> Option<&RemoteState> {
        self.values().next()
    }
    fn as_mut(&mut self) -> Option<&mut RemoteState> {
        self.values_mut().next()
    }
    fn is_none(&self) -> bool {
        self.is_empty()
    }
    fn is_some(&self) -> bool {
        !self.is_empty()
    }
    fn replace(&mut self, state: RemoteState) -> Option<RemoteState> {
        assert!(
            !self.contains_key(&state.pair_id),
            "a single-slot fixture replaced an existing runtime; use insert by pair_id"
        );
        self.insert(state.pair_id.clone(), state)
    }
    fn take(&mut self) -> Option<RemoteState> {
        let pair_id = self.keys().next().cloned()?;
        self.remove(&pair_id)
    }
}

use serde_json::json;
impl Supervisor {
    /// Append one signature field as `<byte-len>:<bytes>`. Because every record
    /// emits a fixed number of fields in a fixed order, length-prefixing makes the
    /// whole catalog signature unambiguous without any record/field separator — so a
    /// title that happens to contain a separator character can't collide two
    /// different catalogs into the same signature (which would silently skip a sync).
    /// Build the full presence snapshot (directory + per-session streaming) together
    /// with a signature that changes iff the snapshot's UI-visible content changes.
    /// The signature is recomputed straight from the store each call, so it can never
    /// drift from reality: a missed dirty-mark only delays propagation (the catalog
    /// tick recomputes it and advertises the new revision), it never desyncs.
    pub fn build_presence_snapshot(
        &self,
        pair_id: &str,
        bridge_instance_id: &str,
    ) -> (serde_json::Value, String) {
        let mut payload = self.light_presence_payload(pair_id, bridge_instance_id);
        payload["catalogEpoch"] = json!(self.host.catalog_epoch());
        let mut signature = String::new();
        if let Some((sessions, sig)) = self.build_sessions_snapshot(pair_id) {
            payload["sessions"] = sessions["sessions"].clone();
            payload["sessionsVersion"] = sessions["version"].clone();
            signature.push_str(&sig);
        }
        if let Some((workspaces, sig)) = self.build_workspaces_snapshot() {
            payload["workspaces"] = workspaces["workspaces"].clone();
            payload["workspacesVersion"] = workspaces["version"].clone();
            signature.push_str(&sig);
        }
        (payload, signature)
    }

    /// Full directory snapshot for the handshake and on-demand `get_presence`
    /// (always complete, so a freshly connected client gets a usable baseline).
    pub fn build_presence_payload(
        &self,
        pair_id: &str,
        bridge_instance_id: &str,
    ) -> serde_json::Value {
        self.build_presence_snapshot(pair_id, bridge_instance_id).0
    }

    /// Sessions-only snapshot for `p.{pair}.state.sessions`.
    ///
    /// Returns `None` when any backing store read fails. A transient SQLite error
    /// must not surface as an *empty* session list — publishing `{"sessions":[]}`
    /// makes the phone's "selected session vanished" heuristic fire and close a
    /// conversation the user is reading (audit 05 L8). On failure the publisher
    /// skips this tick and the phone keeps the previous snapshot.
    pub fn build_sessions_snapshot(&self, pair_id: &str) -> Option<(serde_json::Value, String)> {
        self.host.sessions(pair_id)
    }
    pub fn build_workspaces_snapshot(&self) -> Option<(serde_json::Value, String)> {
        self.host.workspaces()
    }

    /// Liveness-only payload: no directory content. Carries no catalog revision
    /// either; a caller that needs the recovery signal uses
    /// [`Supervisor::presence_heartbeat_payload`].
    pub fn light_presence_payload(
        &self,
        pair_id: &str,
        bridge_instance_id: &str,
    ) -> serde_json::Value {
        let in_control = self.in_control(pair_id);
        json!({
            "online": true,
            "agentAvailable": self.host.agent_available(),
            "pairId": pair_id,
            "bridgeInstanceId": bridge_instance_id,
            // Additive: an older client ignores it, a newer one uses it to tell
            // "connected to the machine" from "allowed to drive the machine".
            "control": if in_control { "active" } else { "standby" },
            // Who holds it, when somebody else does. This is what lets a client ask
            // its user "taking over disconnects <name>" instead of either taking the
            // machine silently or refusing without saying why. Absent for the
            // controller itself: it is not displacing anyone.
            "holder": if in_control { json!(null) } else { self.holder_payload() },
            "lastHeartbeatTs": unix_timestamp(),
        })
    }

    /// The controller's self-declared identity, in the shape the wire carries.
    fn holder_payload(&self) -> serde_json::Value {
        match self.controller_identity() {
            Some(identity) => json!({
                "pairId": self.active_pair(),
                "name": identity.name,
                "kind": identity.kind,
            }),
            // A controller that never said what it is: the client is told the
            // machine is taken, and names it neutrally.
            None => json!({ "pairId": self.active_pair() }),
        }
    }

    /// Tell a device it is no longer the one driving this machine.
    ///
    /// Sent on the superseded device's own `state.` namespace — the platform's
    /// bridge permission already covers it — so the client learns immediately
    /// rather than by inference from a refused command. A client that does not
    /// understand the packet ignores it and finds out the same way it would have
    /// before: its next command comes back `superseded`.
    pub fn publish_standby(&self, pair_id: &str) {
        self.publish_control(pair_id, "standby");
    }

    /// Send one control-state packet on `pair_id`'s own channel.
    ///
    /// Advisory and bounded: the reply path never waits on this notification.
    /// Its completion remains owned by this instance and the host registry.
    pub fn publish_control(&self, pair_id: &str, control: &str) {
        let Some((client, security, bridge_instance_id)) =
            self.state.lock().unwrap().get(pair_id).map(|state| {
                (
                    state.client.clone(),
                    state.security.clone(),
                    state.bridge_instance_id.clone(),
                )
            })
        else {
            // The superseded device is not connected here, so there is nobody to
            // tell. It will read `control: standby` from this machine's next
            // presence whenever it does connect.
            return;
        };
        let subject = format!("p.{pair_id}.state.control");
        let payload = json!({
            "control": control,
            "pairId": pair_id,
            "bridgeInstanceId": bridge_instance_id,
            "lastControlChangeTs": unix_timestamp(),
        });
        let Ok(bytes) = serde_json::to_vec(&payload) else {
            return;
        };
        // The next presence packet carries the same state if this advisory
        // notification is lost. Shutdown still observes its actual completion.
        self.spawn_notice(async move {
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(750),
                crate::secure::publish(&client, &security, subject, bytes),
            )
            .await;
        });
    }

    /// Heartbeat payload: liveness plus the catalog revision.
    ///
    /// The revision is what replaces the old "re-send every unchanged snapshot
    /// every 20s" self-heal. A client compares it against the revision it last
    /// applied; when the advertised one is newer, a pushed snapshot was lost (core
    /// NATS is at-most-once) and the client pulls the catalogue itself. That makes
    /// recovery faster than the timer it replaces (one tick, not 20s) while an
    /// idle, unchanged directory costs nothing but this packet.
    ///
    /// A revision that stays put on an unchanged snapshot is only safe because
    /// `sessions()`/`workspaces()` recompute it from the store on every catalog
    /// tick: the signal is derived from ground truth, not from an accumulating
    /// dirty flag that a missed mark could strand.
    pub fn presence_heartbeat_payload(
        &self,
        pair_id: &str,
        bridge_instance_id: &str,
    ) -> serde_json::Value {
        let mut payload = self.light_presence_payload(pair_id, bridge_instance_id);
        let (epoch, sessions, workspaces) = self.host.catalog_revisions();
        payload["catalogVersion"] = json!({
            "epoch": epoch,
            "sessions": sessions,
            "workspaces": workspaces,
        });
        payload
    }
}

pub fn unix_timestamp() -> u64 {
    // `unwrap_or_default`: a pre-epoch clock is not a reachable failure mode
    // worth an arm — treat it as 0.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::test_support::isolated;
    use super::*;

    /// A supervisor nobody asked to start, built by hand so the process-global
    /// application state is untouched.
    fn stopped_supervisor() -> Arc<Supervisor> {
        isolated()
    }

    fn creds(pair_id: &str, seed: String) -> PairingCreds {
        PairingCreds {
            handshake_version: 2,
            secure: None,
            pair_id: pair_id.to_string(),
            desktop_id: format!("desktop_{pair_id}"),
            nkey_seed: seed,
            user_jwt: "jwt".to_string(),
            nats_url: "nats://127.0.0.1:4222".to_string(),
            nats_ws_url: "ws://127.0.0.1:4222".to_string(),
            jwt_expires_at: 0,
            control_url: None,
        }
    }

    /// The runtime these tasks land on is process-lifetime, so a supervisor
    /// that was never asked to run (or was suspended mid-flight) must schedule
    /// nothing: a stray task would outlive its caller and the pairing that
    /// armed it.
    #[tokio::test]
    async fn a_stopped_or_suspended_supervisor_schedules_no_background_work() {
        let inert = stopped_supervisor();
        inert.spawn(std::future::pending());
        assert!(
            inert.tasks.lock().unwrap().is_empty(),
            "an unrequested supervisor must not schedule"
        );

        inert.start_requested.store(true, Ordering::Release);
        inert.spawn(std::future::pending());
        assert_eq!(
            inert.tasks.lock().unwrap().len(),
            1,
            "a requested supervisor schedules the task"
        );

        inert.suspended.store(true, Ordering::Release);
        inert.spawn(std::future::pending());
        assert_eq!(
            inert.tasks.lock().unwrap().len(),
            1,
            "suspension refuses new work"
        );

        inert.cancel_tasks();
        assert!(
            inert.tasks.lock().unwrap().is_empty(),
            "cancel_tasks drains the registry"
        );
    }

    /// A *confirmed* pairing must latch onto the reused runtime. The phone is
    /// already paired; if a credential refresh rebuilt the runtime's
    /// confirmation flag instead of reusing it, the next claim flow would ask a
    /// paired user to pair again.
    #[test]
    fn a_confirmed_pairing_latches_onto_the_reused_runtime() {
        let supervisor = isolated();
        supervisor.bridge_shared.lock().unwrap().clear();
        let key_pair = nkeys::KeyPair::new_user();
        let creds = creds("pair_latch", key_pair.seed().unwrap().to_string());

        let first = supervisor.shared_runtime(&creds, false, false);
        assert!(
            !first.pairing_confirmed.load(Ordering::Acquire),
            "an unconfirmed start must not report a paired phone"
        );

        let second = supervisor.shared_runtime(&creds, true, true);
        assert!(
            first.reply_slots.shares_cache(&second.reply_slots),
            "a refresh serving the same invitation keeps the reply slots"
        );
        assert!(
            second.pairing_confirmed.load(Ordering::Acquire),
            "the confirmation must survive the generation swap"
        );
        assert_ne!(
            first.bridge_instance_id, second.bridge_instance_id,
            "and the bridge id must still rotate with the generation"
        );
        supervisor.bridge_shared.lock().unwrap().clear();
    }

    /// Credentials whose NKey seed cannot be read describe no invitation, so
    /// the cached runtime — reply single-flight, pairing confirmation, bridge
    /// id — must never be inherited by them: the QR could never authenticate
    /// against it.
    #[test]
    fn an_unreadable_nkey_seed_never_reuses_a_cached_runtime() {
        let supervisor = isolated();
        supervisor.bridge_shared.lock().unwrap().clear();
        let unreadable = creds("pair_unreadable", "not-an-nkey-seed".to_string());

        let first = supervisor.shared_runtime(&unreadable, false, false);
        let second = supervisor.shared_runtime(&unreadable, false, false);
        assert!(
            !first.reply_slots.shares_cache(&second.reply_slots),
            "no provable invitation means a fresh runtime every time"
        );
        assert!(
            !std::sync::Arc::ptr_eq(&first.pairing_confirmed, &second.pairing_confirmed),
            "a previous pairing's confirmation must not carry over"
        );
        supervisor.bridge_shared.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod ownership_tests;
