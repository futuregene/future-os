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
                        eprintln!("{line}");
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
