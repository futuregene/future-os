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
    pub(in crate::remote) state: Mutex<Option<RemoteState>>,
    pub(in crate::remote) bridge_shared: Mutex<Option<BridgeRuntimeShared>>,
    pub(in crate::remote) last_error_code: Mutex<Option<String>>,
    pub(in crate::remote) start_lock: tokio::sync::Mutex<()>,
    pub(in crate::remote) start_requested: AtomicBool,
    pub(in crate::remote) suspended: AtomicBool,
    pub(in crate::remote) resume_recovery_running: AtomicBool,
    pub(in crate::remote) start_retry_running: AtomicBool,
    pub(in crate::remote) start_retry_attempts: AtomicU64,
    pub(in crate::remote) start_retry_since: AtomicU64,
    pub(in crate::remote) start_retry_next_at: AtomicU64,
    pub(in crate::remote) credential_refreshing: AtomicBool,
    pub(in crate::remote) runtime_reconnect_running: AtomicBool,
    pub(in crate::remote) runtime_reconnect_attempts: AtomicU8,
    pub(in crate::remote) runtime_failure_window_started: AtomicU64,
    pub(in crate::remote) web_reconnect_running: AtomicBool,
    pub(in crate::remote) web_reconnect_attempts: AtomicU8,
    pub(in crate::remote) tasks: Mutex<Vec<(futures::future::AbortHandle, Arc<AtomicBool>)>>,
}
pub(in crate::remote) static SUPERVISOR: LazyLock<Supervisor> = LazyLock::new(|| Supervisor {
    access: lifecycle::AccessEpoch::new(),
    state: Mutex::new(None),
    bridge_shared: Mutex::new(None),
    last_error_code: Mutex::new(None),
    start_lock: tokio::sync::Mutex::const_new(()),
    start_requested: AtomicBool::new(false),
    suspended: AtomicBool::new(false),
    resume_recovery_running: AtomicBool::new(false),
    start_retry_running: AtomicBool::new(false),
    start_retry_attempts: AtomicU64::new(0),
    start_retry_since: AtomicU64::new(0),
    start_retry_next_at: AtomicU64::new(0),
    credential_refreshing: AtomicBool::new(false),
    runtime_reconnect_running: AtomicBool::new(false),
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
    /// The live-lane capabilities declared for `pair_id`. An unknown pairing
    /// gets a fresh, off flag: capability is always per connection, never
    /// inherited from whoever connected before.
    pub(in crate::remote) fn coalesce_events(&self, pair_id: &str) -> Arc<AtomicBool> {
        self.bridge_shared
            .lock()
            .unwrap()
            .as_ref()
            .filter(|shared| shared.pair_id == pair_id)
            .map(|shared| shared.coalesce_events.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
    }

    pub(in crate::remote) fn cancel_tasks(&self) {
        for (task, _) in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        self.resume_recovery_running.store(false, Ordering::Release);
        self.start_retry_running.store(false, Ordering::Release);
        self.runtime_reconnect_running
            .store(false, Ordering::Release);
        self.web_reconnect_running.store(false, Ordering::Release);
    }
}

/// State whose correctness spans credential and transport generations. A
/// generation swap must never clear command single-flight replies or pairing
/// confirmation, otherwise a retried command can execute twice and a paired
/// phone can be forced through an unnecessary claim flow.
#[derive(Clone)]
pub(in crate::remote) struct BridgeRuntimeShared {
    pub(in crate::remote) pair_id: String,
    pub(in crate::remote) reply_slots: commands::ReplySlots,
    pub(in crate::remote) pairing_confirmed: Arc<AtomicBool>,
    pub(in crate::remote) coalesce_events: Arc<AtomicBool>,
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
    let reusable = guard.as_ref().is_some_and(|shared| {
        shared.pair_id == pair_id
            && invitation.is_some()
            && shared.invitation.as_ref() == invitation.as_ref()
    });
    if reusable {
        let shared = guard.as_mut().expect("checked above");
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
        coalesce_events: Arc::new(AtomicBool::new(false)),
        next_generation_id: Arc::new(AtomicU64::new(1)),
        handshake: Arc::new(Mutex::new(None)),
        invitation,
    };
    *guard = Some(shared.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::HomeGuard;

    /// A supervisor nobody asked to start, built by hand so the process-global
    /// [`SUPERVISOR`] (which other tests share) is untouched.
    fn stopped_supervisor() -> Supervisor {
        Supervisor {
            access: lifecycle::AccessEpoch::new(),
            state: std::sync::Mutex::new(None),
            bridge_shared: std::sync::Mutex::new(None),
            last_error_code: std::sync::Mutex::new(None),
            start_lock: tokio::sync::Mutex::const_new(()),
            start_requested: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
            resume_recovery_running: AtomicBool::new(false),
            start_retry_running: AtomicBool::new(false),
            start_retry_attempts: AtomicU64::new(0),
            start_retry_since: AtomicU64::new(0),
            start_retry_next_at: AtomicU64::new(0),
            credential_refreshing: AtomicBool::new(false),
            runtime_reconnect_running: AtomicBool::new(false),
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
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
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
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
    }

    /// Credentials whose NKey seed cannot be read describe no invitation, so
    /// the cached runtime — reply single-flight, pairing confirmation, bridge
    /// id — must never be inherited by them: the QR could never authenticate
    /// against it.
    #[test]
    fn an_unreadable_nkey_seed_never_reuses_a_cached_runtime() {
        let _home = HomeGuard::new("remote-unreadable-seed");
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
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
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
    }
}
