//! Host-independent Agent startup and serving.
//!
//! The scoped owner retains the existing singleton through the host's runtime
//! teardown and cleanup, without leaking its lock allocation. Hosts continue to
//! own logging, environment changes, signals, profiling and process exit.
//!
//! Hosts request shutdown through `AgentHandle` and continue polling `serve`
//! until the listener drains. A timeout retains instance ownership until process
//! exit; dropping a serving future is not a successful shutdown.

mod activity;
#[cfg(test)]
mod approval_tests;
pub(crate) mod config;
mod instance;
mod lifecycle;
mod shutdown;
mod startup;
#[cfg(test)]
mod tests;
pub(crate) mod work;

pub use activity::{ActivityUnavailable, AgentActivity};
use anyhow::Result;
pub use config::AgentConfig;
pub use shutdown::{ShutdownReport, ShutdownStatus};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[doc(hidden)]
pub use work::ServiceWork;

/// Cooperative startup cancellation, including checkpoints around synchronous
/// initialization. It never force-kills a thread or releases another owner.
#[derive(Clone, Debug, Default)]
pub struct StartupCancellation(Arc<AtomicBool>);

#[derive(Debug, thiserror::Error)]
#[error("Agent startup cancelled")]
pub struct StartupCancelled;

impl StartupCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            return Err(StartupCancelled.into());
        }
        Ok(())
    }
}

/// Scoped Agent owner. It cannot outlive the file that backs its instance lock.
pub struct AgentService<'lock> {
    config: AgentConfig,
    preparation_started: AtomicBool,
    startup_cleanup_started: AtomicBool,
    work: Arc<work::ServiceWork>,
    handle: parking_lot::Mutex<Option<AgentHandle>>,
    _instance: instance::InstanceGuard<'lock>,
}

/// Evidence that a particular service drained its work and retired its storage.
/// Hosts must also wait for their worker/runtime to exit before restarting.
#[derive(Clone)]
pub struct ServiceRetirement(Arc<work::ServiceWork>);

impl ServiceRetirement {
    pub fn is_complete(&self) -> bool {
        self.0.is_retired()
    }
}

impl AgentService<'_> {
    pub fn retirement(&self) -> ServiceRetirement {
        ServiceRetirement(self.work.clone())
    }

    /// Consume the discovery gate's local-start permission. A race that loses
    /// the actual lock fails before calling the host or initializing storage.
    pub fn with_local_start<T>(
        config: AgentConfig,
        permit: future_rpc::connection::LocalStartPermit,
        host: impl FnOnce(&AgentService<'_>) -> Result<T>,
    ) -> Result<T> {
        anyhow::ensure!(
            config
                .grpc_addr
                .as_deref()
                .is_none_or(|addr| addr.trim().is_empty()
                    || addr
                        .trim()
                        .eq_ignore_ascii_case(future_rpc::transport::AUTO_ENDPOINT)),
            "a local-start permission cannot start an explicit TCP endpoint"
        );
        anyhow::ensure!(
            config.future_home() == permit.future_home(),
            "Agent start permission belongs to another data root"
        );
        permit.recheck()?;
        Self::with_instance(config, host)
    }

    /// Acquire the instance before invoking host code that initializes Agent
    /// data. Metadata failures remain advisory, exactly as in the CLI.
    ///
    /// The host must tear down its runtime and background work before returning.
    /// No logging, signal handlers, environment mutation or runtime are created
    /// here. Sessionless probe/reset commands deliberately do not call this API.
    pub fn with_instance<T>(
        config: AgentConfig,
        host: impl FnOnce(&AgentService<'_>) -> Result<T>,
    ) -> Result<T> {
        config.validate_environment()?;
        let mut file = instance::InstanceFile::open(
            config.future_home.join("agent").join("agent-instance.lock"),
        )?;
        let guard = file.acquire(&config.future_home)?;
        let service = AgentService {
            config,
            preparation_started: AtomicBool::new(false),
            startup_cleanup_started: AtomicBool::new(false),
            work: Arc::new(work::ServiceWork::default()),
            handle: parking_lot::Mutex::new(None),
            _instance: guard,
        };
        work::install(&service.work, &service.config.future_home)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host(&service)));
        let cleanup = service.finish_scope();
        if cleanup.is_err() {
            // fd-lock unlocks in its guard destructor. On uncertain shutdown,
            // intentionally retain BOTH that guard and its backing descriptor
            // until process exit. No other in-process service can take over.
            std::mem::forget(service);
            std::mem::forget(file);
        }
        match result {
            Ok(result) => match (result, cleanup) {
                (Ok(value), Ok(())) => Ok(value),
                (Ok(_), Err(cleanup)) => Err(cleanup),
                (Err(error), Ok(())) => Err(error),
                (Err(error), Err(cleanup)) => Err(error.context(format!("{cleanup:#}"))),
            },
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    fn startup_cleanup(&self) {
        if !self.startup_cleanup_started.swap(true, Ordering::AcqRel) {
            lifecycle::cleanup_windows_sandbox_on_startup();
        }
    }

    fn finish_scope(&self) -> Result<()> {
        let handle = self.handle.lock().clone();
        if let Some(handle) = handle {
            let report = handle.shutdown_blocking(std::time::Duration::from_secs(5));
            anyhow::ensure!(
                report.status == ShutdownStatus::Stopped,
                "Agent failed to stop: {}",
                report.errors.join("; ")
            );
        } else {
            // Failed/cancelled startup can already own a catalog refresh.
            self.work.close();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while self.work.counts() != (0, 0) {
                anyhow::ensure!(
                    std::time::Instant::now() < deadline,
                    "Agent startup background work failed to stop; instance ownership retained"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        self.work.retire()?;
        Ok(())
    }

    /// Existing Windows startup/exit repair around the host's complete runtime
    /// lifetime. Exit repair runs only after the service has drained safely.
    pub fn run_with_cleanup<T>(&self, host: impl FnOnce() -> Result<T>) -> Result<T> {
        self.config.validate_environment()?;
        lifecycle::run_agent_lifecycle(
            || self.startup_cleanup(),
            || {
                let result = host();
                self.finish_scope()?;
                result
            },
            || {
                let initialized = self.handle.lock().is_some();
                if !initialized && self.finish_scope().is_ok() {
                    lifecycle::cleanup_windows_sandbox_on_exit();
                }
            },
        )
    }

    /// Prepare models before entering a Tokio runtime: blocking HTTP clients
    /// used by the registry retain the existing CLI initialization behavior.
    pub fn prepare(&self, cancellation: StartupCancellation) -> Result<PreparedAgent<'_, '_>> {
        cancellation.check()?;
        self.config.validate_environment()?;
        // A borrowed scope can have several callers, but must never build two
        // independent AppStates/stores under the same instance lock. Restart
        // requires a new scope after the host has torn down all old work.
        if self.preparation_started.swap(true, Ordering::AcqRel) {
            anyhow::bail!("Agent service initialization has already been started");
        }
        self.startup_cleanup();
        tracing::info!("Agent startup: loading the model catalogue and credentials");
        // Registered BEFORE the cold registry build: that build spawns the
        // background Future-models fetch, and a fast fetch can land (and fire
        // this callback) before `initialize()` ever constructs the AppState.
        // The holder is filled once the state exists; until then an early
        // fetch's notification is deferred to the registry rebuild below.
        let synced_state: Arc<parking_lot::RwLock<Option<crate::rpc::AppState>>> =
            Arc::new(parking_lot::RwLock::new(None));
        {
            let synced_state = synced_state.clone();
            crate::models::set_future_models_sync_callback(Some(Arc::new(move || {
                let Some(state) = synced_state.read().clone() else {
                    return;
                };
                let _provider_guard = state
                    .control_plane
                    .provider_command_lock
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                *state.model_registry.write() = crate::models::Registry::new();
                state.reconcile_provider_references();
                state.control_plane.publish_provider_config_changed(
                    "future",
                    "models_synced",
                    false,
                    true,
                );
            })));
        }
        let model_registry = Arc::new(parking_lot::RwLock::new(crate::ModelRegistry::new()));
        cancellation.check()?;
        Ok(PreparedAgent {
            owner: self,
            cancellation,
            model_registry,
            synced_state,
        })
    }
}

/// Startup resources tied to the instance owner; no listener is serving yet.
pub struct PreparedAgent<'owner, 'lock> {
    owner: &'owner AgentService<'lock>,
    cancellation: StartupCancellation,
    model_registry: Arc<parking_lot::RwLock<crate::models::Registry>>,
    /// Late-bound AppState for the Future-models sync callback (see
    /// [`AgentService::prepare`]); written by `initialize`, read by the
    /// background-fetch notification path.
    synced_state: Arc<parking_lot::RwLock<Option<crate::rpc::AppState>>>,
}

impl<'owner, 'lock> PreparedAgent<'owner, 'lock> {
    /// Initialize the unchanged engine/store/AppState, then bind the endpoint.
    /// Errors and cancellation drop partial state without reporting readiness.
    pub async fn initialize(self) -> Result<RunningAgent<'owner, 'lock>> {
        let (state, listener) = startup::initialize(
            &self.owner.config,
            self.model_registry,
            &self.cancellation,
            self.owner.work.clone(),
            self.synced_state,
        )
        .await?;
        let ready = AgentReady {
            agent_instance_id: state.agent_instance_id.clone(),
            version: state.welcome_version.clone(),
            endpoint: listener.endpoint(),
        };
        let handle = AgentHandle {
            state: state.clone(),
            shutdown: Arc::new(shutdown::ShutdownShared::default()),
        };
        *self.owner.handle.lock() = Some(handle.clone());
        Ok(RunningAgent {
            handle,
            _owner: self.owner,
            state,
            listener,
            ready,
            cancellation: self.cancellation,
        })
    }
}

/// Local IPC label or the actual TCP address, including an assigned port zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEndpoint {
    Local(String),
    Tcp(std::net::SocketAddr),
}

/// Startup identity notification; the business readiness RPC remains unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentReady {
    pub agent_instance_id: String,
    pub version: String,
    pub endpoint: AgentEndpoint,
}

/// Initialized storage and a bound transport, still borrowing the lock owner.
/// Cloneable host control. It owns no runtime, signal handlers or instance lock.
/// `Stopped` is required before a host can release the scoped instance.
#[derive(Clone)]
pub struct AgentHandle {
    state: crate::rpc::AppState,
    shutdown: Arc<shutdown::ShutdownShared>,
}

pub struct RunningAgent<'owner, 'lock> {
    _owner: &'owner AgentService<'lock>,
    handle: AgentHandle,
    state: crate::rpc::AppState,
    listener: crate::grpc::AgentListener,
    ready: AgentReady,
    cancellation: StartupCancellation,
}

impl RunningAgent<'_, '_> {
    pub fn identity(&self) -> &AgentReady {
        &self.ready
    }

    pub fn handle(&self) -> AgentHandle {
        self.handle.clone()
    }

    /// Notify only after initialization/binding and the server's first
    /// successful poll. Failed startup never sends a Ready identity.
    ///
    /// Keep polling while `AgentHandle::shutdown` drains work. Dropping this
    /// future initiates a bounded drain. Only a completed drain permits restart.
    pub async fn serve(
        self,
        ready: Option<tokio::sync::oneshot::Sender<AgentReady>>,
    ) -> Result<()> {
        let cancellation = self.cancellation;
        cancellation.check()?;
        struct ServerGuard {
            handle: AgentHandle,
            completed: bool,
        }
        impl Drop for ServerGuard {
            fn drop(&mut self) {
                if !self.completed {
                    self.handle.state.service_work.close();
                    self.handle.state.service_work.close_streams();
                    self.handle.state.service_work.stop_listener();
                    self.handle
                        .request_shutdown(std::time::Duration::from_secs(5));
                }
                self.handle
                    .shutdown
                    .server_running
                    .store(false, Ordering::Release);
            }
        }
        let admission = self.state.service_work.enter()?;
        self.handle
            .shutdown
            .server_running
            .store(true, Ordering::Release);
        let mut guard = ServerGuard {
            handle: self.handle,
            completed: false,
        };
        let shutdown = self.state.service_work.listener_shutdown();
        let server = self.listener.serve(self.state, shutdown);
        tokio::pin!(server);
        drop(admission);
        let mut ready = ready.map(|sender| (sender, self.ready));
        let mut server_completed = false;
        let result = futures::future::poll_fn(|cx| {
            let result = std::future::Future::poll(server.as_mut(), cx);
            server_completed = result.is_ready();
            if result.is_pending() {
                if let Some((sender, identity)) = ready.take() {
                    if let Err(error) = cancellation.check() {
                        return std::task::Poll::Ready(Err(error));
                    }
                    if !guard.handle.state.service_work.is_closed() {
                        let _ = sender.send(identity);
                    }
                }
            }
            result
        })
        .await;
        guard.completed = server_completed;
        // The registry-rebuild callback registered at startup now points at a
        // torn-down state; drop it so a late background fetch cannot write
        // into the stopped service. The next service instance re-registers.
        let _ = crate::models::set_future_models_sync_callback(None);
        if result.is_err() {
            // Serving failure is the host's error; it is not evidence that
            // cleanup failed. The independent drain must decide that.
            guard
                .handle
                .request_shutdown(std::time::Duration::from_secs(5));
        }
        result
    }
}
