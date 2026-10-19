use super::*;
pub(in crate::remote) struct ConnectedNats {
    pub(in crate::remote) client: async_nats::Client,
    pub(in crate::remote) health: Arc<NatsHealth>,
}

/// Active remote connection. Holds async-nats client + command/event tasks;
/// on stop, aborts the tasks and drops the client.
pub(in crate::remote) struct RemoteState {
    pub(in crate::remote) security: secure::Transport,
    pub(in crate::remote) generation_id: u64,
    /// Raw client, kept to derive real connection state for [`status`].
    pub(in crate::remote) client: async_nats::Client,
    pub(in crate::remote) nats_health: Arc<NatsHealth>,
    pub(in crate::remote) nats_url: String,
    pub(in crate::remote) pair_id: String,
    pub(in crate::remote) desktop_id: String,
    pub(in crate::remote) desktop_public_key: String,
    pub(in crate::remote) bridge_instance_id: String,
    /// Ordered event queue → single drain task per connection. The drain holds
    /// a clone of the client so the connection stays alive while events are in
    /// flight.
    pub(in crate::remote) event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    pub(in crate::remote) drop_counters: Arc<DropCounters>,
    pub(in crate::remote) event_task: tokio::task::JoinHandle<()>,
    pub(in crate::remote) cmd_task: tokio::task::JoinHandle<()>,
    pub(in crate::remote) transfer_task: tokio::task::JoinHandle<()>,
    pub(in crate::remote) heartbeat_task: tokio::task::JoinHandle<()>,
    pub(in crate::remote) refresh_task: tokio::task::JoinHandle<()>,
    /// `None` outside the test environment, or when the optional test web
    /// server failed to bind. The phone bridge remains available either way.
    pub(in crate::remote) web_task: Option<tokio::task::JoinHandle<()>>,
    /// Test-only web client URL for THIS machine; `None` outside the test
    /// environment or when bind failed.
    pub(in crate::remote) web_url: Option<String>,
    /// Test-only web client URL a phone on the same LAN can reach; `None`
    /// outside the test environment, when bind failed, or without a LAN route.
    pub(in crate::remote) web_lan_url: Option<String>,
    /// The one-shot pairing code issued at start, kept (with its expiry) so the
    /// UI can re-show it after navigation until it expires — no longer a
    /// fire-once value lost the moment you switch views.
    pub(in crate::remote) pairing_code: Option<String>,
    pub(in crate::remote) pairing_code_expires_at: Option<i64>,
    /// New pairings remain pending until the client and bridge complete the
    /// signed application-level handshake.
    pub(in crate::remote) pairing_confirmed: Arc<AtomicBool>,
}

/// The sole Desktop-process owner of remote intent, access epoch, runtime,
/// recovery budgets and background scheduling. UI only reads its projection.
pub(in crate::remote) struct Supervisor {
    pub(in crate::remote) access: lifecycle::AccessEpoch,
    /// One runtime per paired device, keyed by `pair_id`.
    ///
    /// Every slot keeps its own NATS connection, presence, credential refresh
    /// and command loop, so a device losing its link — or losing its JWT — is
    /// that device's problem and never the other one's. What the slots do *not*
    /// share is the right to drive this machine; see [`Supervisor::active`].
    pub(in crate::remote) state: Mutex<HashMap<String, RemoteState>>,
    pub(in crate::remote) bridge_shared: Mutex<HashMap<String, BridgeRuntimeShared>>,
    /// The one device allowed to drive this machine.
    ///
    /// Mirrored from the pairing book, and only ever moved by an explicit claim
    /// — a confirmed pairing handshake or a `takeover` request. It is
    /// deliberately *not* moved by a reconnect: a device that was superseded
    /// must come back as a standby, or the two devices would take turns
    /// evicting each other for as long as either one keeps reconnecting.
    pub(in crate::remote) active: Mutex<Option<String>>,
    /// The devices this process has a credential for.
    ///
    /// Kept in memory so [`Supervisor::in_control`] — which every command and
    /// every presence packet reads — never touches the filesystem. It is
    /// written by start (the slots it served), by a slot being added, and by an
    /// unpair.
    pub(in crate::remote) known: Mutex<std::collections::HashSet<String>>,
    pub(in crate::remote) last_error_code: Mutex<Option<String>>,
    pub(in crate::remote) start_lock: tokio::sync::Mutex<()>,
    pub(in crate::remote) start_requested: AtomicBool,
    /// Whether a start with nothing paired mints an invitation.
    ///
    /// The GUI's switch does not: the user asks for a connection by adding one
    /// device at a time, and a machine with no devices is simply on and empty.
    /// The headless surface and the pair-a-device entry points do, because there
    /// the invitation *is* what the user asked for.
    pub(in crate::remote) invite_when_empty: AtomicBool,
    pub(in crate::remote) suspended: AtomicBool,
    pub(in crate::remote) resume_recovery_running: AtomicBool,
    pub(in crate::remote) start_retry_running: AtomicBool,
    pub(in crate::remote) start_retry_attempts: AtomicU64,
    pub(in crate::remote) start_retry_since: AtomicU64,
    pub(in crate::remote) start_retry_next_at: AtomicU64,
    pub(in crate::remote) credential_refreshing: AtomicBool,
    pub(in crate::remote) runtime_reconnect_attempts: AtomicU8,
    pub(in crate::remote) runtime_failure_window_started: AtomicU64,
    pub(in crate::remote) web_reconnect_running: AtomicBool,
    pub(in crate::remote) web_reconnect_attempts: AtomicU8,
    pub(in crate::remote) tasks: Mutex<Vec<(futures::future::AbortHandle, Arc<AtomicBool>)>>,
}
pub(in crate::remote) static SUPERVISOR: LazyLock<Supervisor> = LazyLock::new(|| Supervisor {
    access: lifecycle::AccessEpoch::new(),
    state: Mutex::new(HashMap::new()),
    bridge_shared: Mutex::new(HashMap::new()),
    active: Mutex::new(None),
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
    tasks: Mutex::new(Vec::new()),
});
impl Supervisor {
    pub(in crate::remote) fn spawn(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let mut tasks = self.tasks.lock().unwrap();
        if !self.start_requested.load(Ordering::Acquire) || self.suspended.load(Ordering::Acquire) {
            return;
        }
        tasks.retain(|(_, done)| !done.load(Ordering::Acquire));
        let (abort, registration) = futures::future::AbortHandle::new_pair();
        let done = Arc::new(AtomicBool::new(false));
        tasks.push((abort, done.clone()));
        crate::runtime::spawn(async move {
            let _ = futures::future::Abortable::new(future, registration).await;
            done.store(true, Ordering::Release);
        });
    }
    /// What the client on `pair_id` said it was.
    ///
    /// An unknown pairing gets an empty slot rather than the previous client's
    /// description: a device that connected before must not be described on a
    /// pairing it is not part of.
    pub(in crate::remote) fn client_identity(
        &self,
        pair_id: &str,
    ) -> Option<Arc<Mutex<Option<ClientIdentity>>>> {
        self.bridge_shared
            .lock()
            .unwrap()
            .get(pair_id)
            .map(|shared| shared.client_identity.clone())
    }

    /// The live-lane capabilities declared for `pair_id`. Always off for a
    /// connection that has not declared them: capability is never inherited
    /// from whoever connected before.
    pub(in crate::remote) fn coalesce_events(&self, pair_id: &str) -> Arc<AtomicBool> {
        self.shared_or_insert(pair_id).coalesce_events
    }

    /// Whether the client on `pair_id` asked for the catalog's recency packets
    /// instead of a whole session list per second of streaming. Same per-pairing
    /// rule as [`Supervisor::coalesce_events`].
    pub(in crate::remote) fn catalog_activity(&self, pair_id: &str) -> Arc<AtomicBool> {
        self.shared_or_insert(pair_id).catalog_activity
    }

    /// Whether the connection on `pair_id` declared the lean feed.
    pub(in crate::remote) fn lean_events(&self, pair_id: &str) -> Arc<AtomicBool> {
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
    pub(in crate::remote) fn in_control(&self, pair_id: &str) -> bool {
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
    pub(in crate::remote) fn held_by_another(&self, pair_id: &str) -> bool {
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
    pub(in crate::remote) fn controller_identity(&self) -> Option<ClientIdentity> {
        let active = self.active_pair()?;
        let slot = self.client_identity(&active)?;
        let identity = slot.lock().unwrap().clone();
        identity
    }

    /// Forget every device and the controller — a machine-wide stop.
    pub(in crate::remote) fn reset_devices(&self) {
        self.known.lock().unwrap().clear();
        *self.active.lock().unwrap() = None;
    }

    pub(in crate::remote) fn remember_device(&self, pair_id: &str) {
        self.known.lock().unwrap().insert(pair_id.to_string());
    }

    pub(in crate::remote) fn forget_device(&self, pair_id: &str) {
        self.known.lock().unwrap().remove(pair_id);
    }

    /// The device in control, if one claimed it.
    pub(in crate::remote) fn active_pair(&self) -> Option<String> {
        self.active.lock().unwrap().clone()
    }

    /// The device that actually drives the machine right now.
    ///
    /// Same rule as [`Supervisor::in_control`], in the shape a publisher needs:
    /// the claimed pointer, else the only connected device. `None` when several
    /// are connected and none has been given the machine — in which case there
    /// is nobody to publish to, which is the honest answer.
    pub(in crate::remote) fn controlling_pair(&self) -> Option<String> {
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
    pub(in crate::remote) fn restore_active(&self, pair_id: Option<String>) {
        *self.active.lock().unwrap() = pair_id;
    }

    /// Move control to `pair_id`. Returns the device that lost it, if any.
    ///
    /// The persisted pointer is written before returning: a control that a
    /// restart could not reproduce would hand the machine to a different device
    /// after a crash, which is exactly the decision the user made and must not
    /// be lost. Callers on a reply path use [`Supervisor::claim_control_soon`].
    pub(in crate::remote) fn claim_control(&self, pair_id: &str) -> Option<String> {
        let (changed, previous) = self.move_control(pair_id);
        if changed {
            if let Err(error) = pairing::set_active(pair_id) {
                eprintln!("remote: could not persist the controlling device: {error}");
            }
        }
        previous
    }

    /// Move control now, write it down on a thread of its own.
    ///
    /// For the handshake and takeover paths: a reply must never wait on the
    /// filesystem, and a claim issued from a handshake handler must never block
    /// a single-threaded executor. The in-memory pointer is what every command
    /// and every publisher reads, and it moves immediately.
    pub(in crate::remote) fn claim_control_soon(&self, pair_id: &str) -> Option<String> {
        let (changed, previous) = self.move_control(pair_id);
        if changed {
            let persisted = pair_id.to_string();
            std::thread::spawn(move || {
                if let Err(error) = pairing::set_active(&persisted) {
                    eprintln!("remote: could not persist the controlling device: {error}");
                }
            });
        }
        previous
    }

    /// The in-memory half: `(did it move, who held it before)`.
    ///
    /// A move from "nobody" counts: that is the case a machine with one device
    /// hits at start, and the one that must still be written down.
    fn move_control(&self, pair_id: &str) -> (bool, Option<String>) {
        let mut active = self.active.lock().unwrap();
        if active.as_deref() == Some(pair_id) {
            return (false, None);
        }
        (true, active.replace(pair_id.to_string()))
    }

    /// Forget the pointer, e.g. when the controlling device is unpaired.
    pub(in crate::remote) fn release_control(&self, pair_id: &str) {
        let mut active = self.active.lock().unwrap();
        if active.as_deref() == Some(pair_id) {
            *active = None;
        }
    }

    pub(in crate::remote) fn cancel_tasks(&self) {
        for (task, _) in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
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
pub(in crate::remote) struct BridgeRuntimeShared {
    pub(in crate::remote) reply_slots: commands::ReplySlots,
    pub(in crate::remote) pairing_confirmed: Arc<AtomicBool>,
    pub(in crate::remote) coalesce_events: Arc<AtomicBool>,
    /// Whether this client may be sent recency packets instead of a whole
    /// session list whenever a run writes its thread's activity timestamp. Lives
    /// with the other capability flags and for the same reason: a client that
    /// cannot apply a packet must keep receiving whole snapshots.
    pub(in crate::remote) catalog_activity: Arc<AtomicBool>,
    /// Whether this connection declared the lean feed. Per pairing like the
    /// others: trimming one device's history to fields another device asked to
    /// skip would be a silent narrowing of what it receives.
    pub(in crate::remote) lean_events: Arc<AtomicBool>,
    /// What the connected client said it was, until a new pairing replaces it.
    ///
    /// Spans generations like the fields above, and for the same reason: a
    /// credential refresh is the *same* device reconnecting, so the UI must not
    /// blink to "unknown device" every time a JWT is renewed. A re-minted
    /// invitation does start a new runtime, which is right — a different device
    /// may claim it.
    pub(in crate::remote) client_identity: Arc<Mutex<Option<ClientIdentity>>>,
    pub(in crate::remote) bridge_instance_id: String,
    pub(in crate::remote) drop_counters: Arc<DropCounters>,
    pub(in crate::remote) next_generation_id: Arc<AtomicU64>,
    pub(in crate::remote) handshake: Arc<Mutex<Option<commands::HandshakeState>>>,
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
pub(in crate::remote) struct InvitationKey {
    desktop_public_key: String,
    secure_public_key: String,
}

impl InvitationKey {
    /// `None` when the NKey seed cannot be read at all — the bridge can then
    /// serve no invitation, so a cached runtime must never be reused for it.
    fn of(creds: &pairing::PairingCreds) -> Option<Self> {
        Some(Self {
            desktop_public_key: pairing::public_key(creds).ok()?,
            secure_public_key: creds
                .secure
                .as_ref()
                .map(|identity| identity.public_key.clone())
                .unwrap_or_default(),
        })
    }
}

pub(in crate::remote) fn shared_runtime(
    creds: &pairing::PairingCreds,
    pairing_confirmed: bool,
    rotate_epoch: bool,
) -> BridgeRuntimeShared {
    let pair_id = creds.pair_id.as_str();
    let invitation = InvitationKey::of(creds);
    let mut guard = SUPERVISOR.bridge_shared.lock().unwrap();
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
        next_generation_id: Arc::new(AtomicU64::new(1)),
        handshake: Arc::new(Mutex::new(None)),
        invitation,
    };
    guard.insert(pair_id.to_string(), shared.clone());
    shared
}

// Recovery budgets belong to the supervisor and survive transport replacement.
pub(in crate::remote) const MAX_RUNTIME_RECONNECT_ATTEMPTS: u8 = 3;
pub(in crate::remote) const RUNTIME_FAILURE_WINDOW_MS: u64 = 10 * 60 * 1_000;
/// Do not forgive a crash-loop merely because a replacement generation stayed
/// alive for one poll. Only a sustained healthy minute resets the budget.
#[cfg(not(test))]
pub(in crate::remote) const RUNTIME_HEALTHY_RESET_SECS: u8 = 60;
pub(in crate::remote) const MAX_WEB_RECONNECT_ATTEMPTS: u8 = 3;

/// Single-slot view of the runtime map, for tests.
///
/// Production code addresses one device at a time by `pair_id`, because that is
/// the whole point of the map. The bridge tests predate multi-device and each
/// drives exactly one pairing, so they keep their `Option`-shaped vocabulary
/// through this adapter instead of every assertion learning about the map —
/// which would obscure what they are asserting about a single connection.
#[cfg(test)]
pub(in crate::remote) trait SingleSlotMap {
    fn as_ref(&self) -> Option<&RemoteState>;
    fn as_mut(&mut self) -> Option<&mut RemoteState>;
    fn is_none(&self) -> bool;
    fn is_some(&self) -> bool;
    fn replace(&mut self, state: RemoteState) -> Option<RemoteState>;
    fn take(&mut self) -> Option<RemoteState>;
}

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::HomeGuard;

    /// A supervisor nobody asked to start, built by hand so the process-global
    /// [`SUPERVISOR`] (which other tests share) is untouched.
    fn stopped_supervisor() -> Supervisor {
        Supervisor {
            access: lifecycle::AccessEpoch::new(),
            state: std::sync::Mutex::new(std::collections::HashMap::new()),
            bridge_shared: std::sync::Mutex::new(std::collections::HashMap::new()),
            active: std::sync::Mutex::new(None),
            known: std::sync::Mutex::new(std::collections::HashSet::new()),
            last_error_code: std::sync::Mutex::new(None),
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
            tasks: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn creds(pair_id: &str, seed: String) -> pairing::PairingCreds {
        pairing::PairingCreds {
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
        let _home = HomeGuard::new("remote-runtime-latch");
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
        let key_pair = nkeys::KeyPair::new_user();
        let creds = creds("pair_latch", key_pair.seed().unwrap().to_string());

        let first = shared_runtime(&creds, false, false);
        assert!(
            !first.pairing_confirmed.load(Ordering::Acquire),
            "an unconfirmed start must not report a paired phone"
        );

        let second = shared_runtime(&creds, true, true);
        assert!(
            std::sync::Arc::ptr_eq(&first.reply_slots, &second.reply_slots),
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
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
    }

    /// Credentials whose NKey seed cannot be read describe no invitation, so
    /// the cached runtime — reply single-flight, pairing confirmation, bridge
    /// id — must never be inherited by them: the QR could never authenticate
    /// against it.
    #[test]
    fn an_unreadable_nkey_seed_never_reuses_a_cached_runtime() {
        let _home = HomeGuard::new("remote-unreadable-seed");
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
        let unreadable = creds("pair_unreadable", "not-an-nkey-seed".to_string());

        let first = shared_runtime(&unreadable, false, false);
        let second = shared_runtime(&unreadable, false, false);
        assert!(
            !std::sync::Arc::ptr_eq(&first.reply_slots, &second.reply_slots),
            "no provable invitation means a fresh runtime every time"
        );
        assert!(
            !std::sync::Arc::ptr_eq(&first.pairing_confirmed, &second.pairing_confirmed),
            "a previous pairing's confirmation must not carry over"
        );
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
    }
}
