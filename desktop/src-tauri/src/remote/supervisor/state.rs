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
    pub(in crate::remote) bridge_instance_id: String,
    pub(in crate::remote) drop_counters: Arc<DropCounters>,
    pub(in crate::remote) next_generation_id: Arc<AtomicU64>,
    pub(in crate::remote) handshake: Arc<Mutex<Option<commands::HandshakeState>>>,
}

pub(in crate::remote) fn shared_runtime(
    pair_id: &str,
    pairing_confirmed: bool,
    rotate_epoch: bool,
) -> BridgeRuntimeShared {
    let mut guard = SUPERVISOR.bridge_shared.lock().unwrap();
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
        handshake: Arc::new(Mutex::new(None)),
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
