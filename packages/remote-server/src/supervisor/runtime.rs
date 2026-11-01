//! Explicit execution and platform capabilities supplied by the host.
use super::*;
use std::time::Duration;

/// All intervals must be nonzero. Application tests supply their original fast
/// cadence and disable automatic recovery explicitly, independently of cfg(test).
#[derive(Clone, Copy)]
pub struct RuntimeOptions {
    pub publisher: PublisherTiming,
    pub refresh_tick: Duration,
    pub expired_retry: Duration,
    pub transfer_cleanup: Duration,
    pub reply_ttl: Duration,
    pub automatic_recovery: bool,
    pub revoke_cleanup: bool,
}
impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            publisher: Default::default(),
            refresh_tick: Duration::from_secs(15),
            expired_retry: Duration::from_secs(3),
            transfer_cleanup: Duration::from_secs(60),
            reply_ttl: Duration::from_secs(600),
            automatic_recovery: true,
            revoke_cleanup: true,
        }
    }
}
pub struct ValidationSurface {
    pub task: BoxFuture<'static, ()>,
    pub url: String,
    pub lan_url: Option<String>,
}
pub trait TaskExecutor: Send + Sync {
    fn spawn(&self, future: BoxFuture<'static, ()>);
    /// Execute off the caller thread; completion is owned by the actual job.
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>);
    fn options(&self) -> RuntimeOptions {
        RuntimeOptions::default()
    }
    fn connection_options(&self) -> crate::connection::ConnectionOptions {
        Default::default()
    }
    fn attach_events(&self) {}
    fn validation_enabled(&self) -> bool {
        false
    }
    fn bind_validation(&self) -> BoxFuture<'_, Result<ValidationSurface, crate::Error>> {
        Box::pin(async { Err(crate::Error::Message("remote_validation_disabled".into())) })
    }
    fn gzip_override(&self) -> bool {
        false
    }
    fn is_stopping(&self) -> bool {
        false
    }
}
#[cfg(any(test, feature = "test-support"))]
pub type ReadinessPause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct RuntimeHooks {
    pub readiness_pause: Mutex<Option<ReadinessPause>>,
    pub inject_build_failure: std::sync::atomic::AtomicUsize,
    /// How many times `add` built a bridge for the invitation it minted. The
    /// retry is otherwise only visible as elapsed time, which a broken broker
    /// spends in the same place whether or not the loop went round again.
    pub add_attempts: std::sync::atomic::AtomicU8,
    /// Let the fixture run the production recovery workers.
    ///
    /// The fixture keeps `automatic_recovery` off so a test's own `start_once`
    /// never races a background rebuild it did not ask for. A test that is
    /// *about* that path — "was this generation replaced after its credential
    /// was refused" — has nowhere else to observe it, so it raises this and the
    /// watch, its attempt budget and the rebuild all run as they do in the app,
    /// with the fixture's timings. Serialized by `HomeGuard` like every other
    /// process-global switch here, and lowered again by the test that raised it.
    pub recovery_enabled: std::sync::atomic::AtomicBool,
}
impl Supervisor {
    pub fn empty(&self) -> crate::types::RemoteStatus {
        crate::types::empty(self.host.agent_available())
    }
    pub fn status(self: &Arc<Self>) -> crate::types::RemoteStatus {
        self.status_focused(None)
    }
    pub fn status_focused(self: &Arc<Self>, prefer: Option<&str>) -> crate::types::RemoteStatus {
        let projection = self.project_status(prefer, self.executor.validation_enabled());
        if let Some(pair) = projection.retry_web {
            self.spawn_web_reconnect(pair);
        }
        projection.status
    }
    pub async fn connect_nats(
        &self,
        creds: &PairingCreds,
        refreshed: bool,
    ) -> Result<ConnectedNats, crate::Error> {
        crate::connection::connect_nats(creds, refreshed, &self.executor.connection_options()).await
    }
    /// Record the device a pairing's client says it is, retiring any older
    /// pairing of the same device.
    ///
    /// Called from the command loop on `secure_ready` — the first place the
    /// desktop learns which physical device this is — so the work is detached:
    /// unpairing is a store write, a bridge teardown and a revoke, none of which
    /// the client's handshake should wait for.
    pub fn note_client_device(self: &Arc<Self>, pair_id: &str, device_id: &str) {
        let device = device_id.trim();
        if device.is_empty() {
            return;
        }
        let live: Vec<String> = self.state.lock().unwrap().keys().cloned().collect();
        let duplicate = {
            let mut devices = self.client_devices.lock().unwrap();
            // A stable order, so "older" means the pairing that was declared
            // first. Without it both pairings of one device would take turns
            // retiring each other for as long as the phone reconnects: whichever
            // declaration arrived last would be the one kept, and the other would
            // be unpaired on its own reconnect.
            let seq = match devices.get(pair_id) {
                Some((_, seq)) => *seq,
                None => self.client_device_seq.fetch_add(1, Ordering::AcqRel),
            };
            devices.insert(pair_id.to_string(), (device.to_string(), seq));
            duplicate_pairing_in(pair_id, device, seq, &devices, live.into_iter())
        };
        let Some(older) = duplicate else {
            return;
        };
        // Said out loud: this removes a device row the user may remember
        // pairing, and the log is the only place that explains why.
        future_remote_protocol::remote_log!(
            "remote: the same device is already paired [{older}]; dropping that pairing instead of serving it twice [PA001]"
        );
        let server = self.clone();
        self.spawn(async move {
            if let Err(error) = server.unpair_slot(&older).await {
                future_remote_protocol::remote_log!(
                    "remote: could not drop the duplicate pairing [{older}]: {error}"
                );
            }
        });
    }

    pub fn spawn_revoke_cleanup(self: &Arc<Self>) {
        if self.closed.load(Ordering::Acquire) || !self.executor.options().revoke_cleanup {
            return;
        }
        let mut worker = self.maintenance.lock().unwrap();
        if self.closed.load(Ordering::Acquire)
            || worker.as_ref().is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let host = self.host.clone();
        let failures = self.start_failures.clone();
        // No strong supervisor capture: this compensation worker must not keep
        // its owner alive after the host drops the last runtime handle.
        let (handle, future) = tasks::tracked(Box::pin(async move {
            loop {
                if let Err(error) = host.retry_revokes().await {
                    if let Some(line) = failures.record("local", error) {
                        future_remote_protocol::remote_log!("{line}");
                    }
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }));
        *worker = Some(handle);
        self.executor.spawn(future);
    }
    pub async fn shutdown(self: &Arc<Self>, budget: Duration) -> Result<(), crate::Error> {
        let started = std::time::Instant::now();
        self.closed.store(true, Ordering::Release);
        self.stop();
        for task in self.generation_tasks.lock().unwrap().iter() {
            task.abort();
        }
        if let Some(task) = self.maintenance.lock().unwrap().take() {
            task.abort();
            self.retiring.lock().unwrap().push(task);
        }
        self.wait_tasks(budget.saturating_sub(started.elapsed()))
            .await
    }
    pub(crate) fn spawn_notice(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let (handle, future) = tasks::tracked(Box::pin(future));
        let mut pending = self.retiring.lock().unwrap();
        pending.retain(|task| !task.is_finished());
        pending.push(handle);
        self.executor.spawn(future);
    }
    pub(crate) fn spawn_command(&self, epoch: u64, future: BoxFuture<'static, ()>) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let _ = self.access.commit(epoch, || {
            let (handle, future) = tasks::tracked(future);
            let mut tasks = self.tasks.lock().unwrap();
            tasks.retain(|task| !task.is_finished());
            tasks.push(handle);
            self.executor.spawn(future);
        });
    }
}

impl Supervisor {
    /// Generation tasks remain observable even after their socket/queue was
    /// replaced. Old event drains finish in order; shutdown cancels the rest.
    pub(crate) fn spawn_generation_for(
        &self,
        epoch: u64,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> TaskHandle {
        let (handle, future) = tasks::tracked(Box::pin(future));
        if self.closed.load(Ordering::Acquire) {
            drop(future);
            return handle;
        }
        let owned = handle.clone();
        // Cancellation and registration are one decision, so a host result that
        // arrives after stop cannot install or run a new generation task.
        let _ = self.access.commit(epoch, || {
            let mut pending = self.generation_tasks.lock().unwrap();
            pending.retain(|task| !task.is_finished());
            pending.push(owned);
            self.executor.spawn(future);
        });
        handle
    }
    pub(crate) fn cancel_generations(&self) {
        for task in self.generation_tasks.lock().unwrap().iter() {
            task.abort();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn spawn_fixture_task(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> TaskHandle {
        self.spawn_generation_for(self.access.current(), future)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Supervisor {
    /// A process-wide application fixture can reopen its test singleton only
    /// after the host registry has observed every old job being destroyed.
    /// Normal library builds never expose a way to reopen a closed instance.
    pub fn reset_after_shutdown_for_test(&self) {
        if !self.closed.load(Ordering::Acquire) {
            return;
        }
        assert!(self.state.lock().unwrap().is_empty());
        for jobs in [&self.tasks, &self.retiring, &self.generation_tasks] {
            let mut jobs = jobs.lock().unwrap();
            assert!(
                jobs.iter().all(TaskHandle::is_finished),
                "reset raced live remote work"
            );
            jobs.clear();
        }
        let mut native = self.native_tasks.lock().unwrap();
        assert!(
            native.iter().all(NativeHandle::is_finished),
            "reset raced native remote work"
        );
        native.clear();
        let mut maintenance = self.maintenance.lock().unwrap();
        assert!(maintenance.as_ref().is_none_or(TaskHandle::is_finished));
        *maintenance = None;
        self.access
            .invalidate(|| self.closed.store(false, Ordering::Release));
    }
}

/// Another live pairing of `device_id`, or `None`.
///
/// Pure so the rule can be tested without a running bridge: `devices` is the
/// record of what each pairing declared, `live` is the pairings this machine is
/// currently serving. A pairing that is not live is ignored — a stale record for
/// a device the user removed must not make the *next* pairing of that device
/// unpair nothing.
fn duplicate_pairing_in(
    pair_id: &str,
    device_id: &str,
    seq: u64,
    devices: &HashMap<String, (String, u64)>,
    live: impl Iterator<Item = String>,
) -> Option<String> {
    live.filter(|other| other != pair_id).find(|other| {
        devices
            .get(other)
            .is_some_and(|(device, other_seq)| device == device_id && *other_seq < seq)
    })
}

#[cfg(test)]
mod device_dedup_tests {
    use super::*;

    /// Pairing, device, and the order its declaration arrived in.
    fn map(pairs: &[(&str, &str, u64)]) -> HashMap<String, (String, u64)> {
        pairs
            .iter()
            .map(|(pair, device, seq)| (pair.to_string(), (device.to_string(), *seq)))
            .collect()
    }

    #[test]
    fn a_second_pairing_of_the_same_device_names_the_first() {
        let devices = map(&[("pair_old", "dev_phone", 0), ("pair_new", "dev_phone", 1)]);
        let live = vec!["pair_old".to_string(), "pair_new".to_string()];
        assert_eq!(
            duplicate_pairing_in("pair_new", "dev_phone", 1, &devices, live.into_iter()),
            Some("pair_old".to_string())
        );
    }

    /// The older declaration never retires the newer one.
    ///
    /// Both pairings of a device stay live while the phone reconnects, and each
    /// of them calls this on its own `secure_ready`. Comparing the declaration
    /// order — not "somebody else has it" — is what keeps them from unpairing
    /// each other in turns.
    #[test]
    fn the_older_declaration_never_retires_the_newer_one() {
        let devices = map(&[("pair_old", "dev_phone", 0), ("pair_new", "dev_phone", 1)]);
        let live = vec!["pair_old".to_string(), "pair_new".to_string()];
        assert_eq!(
            duplicate_pairing_in("pair_old", "dev_phone", 0, &devices, live.into_iter()),
            None
        );
    }

    #[test]
    fn a_different_device_is_not_a_duplicate() {
        let devices = map(&[("pair_old", "dev_phone", 0), ("pair_new", "dev_tablet", 1)]);
        let live = vec!["pair_old".to_string(), "pair_new".to_string()];
        assert_eq!(
            duplicate_pairing_in("pair_new", "dev_tablet", 1, &devices, live.into_iter()),
            None
        );
    }

    #[test]
    fn a_record_of_a_pairing_this_machine_is_not_serving_is_ignored() {
        // The user unpaired the old row; its record lingers, but that pairing is
        // not live, so nothing is "already paired".
        let devices = map(&[("pair_gone", "dev_phone", 0), ("pair_new", "dev_phone", 1)]);
        let live = vec!["pair_new".to_string()];
        assert_eq!(
            duplicate_pairing_in("pair_new", "dev_phone", 1, &devices, live.into_iter()),
            None
        );
    }

    #[test]
    fn the_pairing_itself_is_never_its_own_duplicate() {
        let devices = map(&[("pair_new", "dev_phone", 1)]);
        let live = vec!["pair_new".to_string()];
        assert_eq!(
            duplicate_pairing_in("pair_new", "dev_phone", 1, &devices, live.into_iter()),
            None
        );
    }
}
