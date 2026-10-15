//! Shared GUI/headless owner. The native worker holds the service scope and
//! its singleton lock through runtime teardown; clients retain only handles.
use future_agent::service::{
    AgentConfig, AgentHandle, AgentService, ShutdownStatus, StartupCancellation, StartupCancelled,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed(String),
}

struct State {
    phase: Phase,
    owned: bool,
    finished: bool,
    cancel: StartupCancellation,
    probe_cancel: Arc<tokio::sync::Notify>,
    handle: Option<AgentHandle>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
#[derive(Clone)]
pub(crate) struct AgentHost(Arc<Shared>);
impl Default for AgentHost {
    fn default() -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                phase: Phase::Idle,
                owned: false,
                finished: true,
                cancel: StartupCancellation::default(),
                probe_cancel: Arc::new(tokio::sync::Notify::new()),
                handle: None,
            }),
            changed: Condvar::new(),
        }))
    }
}
static HOST: std::sync::LazyLock<AgentHost> = std::sync::LazyLock::new(AgentHost::default);
#[cfg(test)]
mod tests;
pub(crate) fn host() -> &'static AgentHost {
    &HOST
}

impl AgentHost {
    pub(crate) fn phase(&self) -> Phase {
        self.0.state.lock().unwrap().phase.clone()
    }
    pub(crate) fn owns_agent(&self) -> bool {
        self.0.state.lock().unwrap().owned
    }
    pub(crate) fn activity(
        &self,
    ) -> Result<Option<future_agent::service::AgentActivity>, crate::AppError> {
        let state = self.0.state.lock().unwrap();
        if !state.owned {
            return Ok(None);
        }
        state
            .handle
            .as_ref()
            .ok_or("Agent is still starting")?
            .activity_snapshot()
            .map(Some)
            .map_err(|e| e.to_string().into())
    }

    pub(crate) fn start(&self, configured: String) -> Result<(), crate::AppError> {
        if crate::desktop_shutdown::is_stopping() {
            return Err("Desktop is stopping".into());
        }
        {
            let mut state = self.0.state.lock().unwrap();
            match &state.phase {
                Phase::Ready => return Ok(()),
                Phase::Starting => {}
                Phase::Idle | Phase::Stopped => {
                    state.phase = Phase::Starting;
                    state.finished = false;
                    state.owned = false;
                    state.handle = None;
                    state.cancel = StartupCancellation::default();
                    state.probe_cancel = Arc::new(tokio::sync::Notify::new());
                    let host = self.clone();
                    if let Err(error) = std::thread::Builder::new()
                        .name("embedded-agent".into())
                        .spawn(move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    host.run(&configured)
                                }));
                            let result = result
                                .unwrap_or_else(|_| Err("Embedded Agent worker panicked".into()));
                            let mut state = host.0.state.lock().unwrap();
                            if !matches!(state.phase, Phase::Failed(_)) {
                                state.phase = match result {
                                    Ok(()) if !state.owned && state.phase == Phase::Ready => {
                                        Phase::Ready
                                    }
                                    Ok(()) => Phase::Stopped,
                                    Err(error) => Phase::Failed(error.to_string()),
                                };
                            }
                            state.finished = true;
                            crate::host_environment::report(&format!(
                                "Agent host finished owned={} phase={:?}",
                                state.owned, state.phase
                            ));
                            host.0.changed.notify_all();
                        })
                    {
                        state.phase = Phase::Failed(error.to_string());
                        state.finished = true;
                    }
                }
                _ => {
                    return Err(
                        "Agent host cannot start before a successful stop; restart Desktop".into(),
                    )
                }
            }
        }
        let deadline = Instant::now() + START_TIMEOUT;
        let mut state = self.0.state.lock().unwrap();
        loop {
            match &state.phase {
                Phase::Ready => return Ok(()),
                Phase::Starting => {}
                Phase::Failed(error) => return Err(error.clone().into()),
                _ => return Err("Agent startup was cancelled".into()),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.cancel.cancel();
                state.probe_cancel.notify_one();
                state.phase = Phase::Failed("Agent startup timed out; restart Desktop".into());
                return Err("Agent startup timed out; restart Desktop".into());
            }
            state = self.0.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn run(&self, configured: &str) -> Result<(), crate::AppError> {
        crate::install_rustls_provider();
        let mode = std::env::var("FUTURE_DESKTOP_AGENT_MODE").unwrap_or_else(|_| "auto".into());
        if !matches!(mode.as_str(), "auto" | "external") {
            return Err("FUTURE_DESKTOP_AGENT_MODE must be auto or external".into());
        }
        let (cancel, probe_cancel) = {
            let state = self.0.state.lock().unwrap();
            if state.phase != Phase::Starting {
                return Ok(());
            }
            (state.cancel.clone(), state.probe_cancel.clone())
        };
        let probe = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let selection = probe.block_on(async {
            tokio::select! {
                biased;
                _ = probe_cancel.notified() => Ok(None),
                result = future_rpc::connection::select_agent(
                    configured,
                    crate::build_info::VERSION,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                ) => result.map(Some),
            }
        });
        drop(probe);
        let selection = match selection {
            Ok(selection) => selection,
            Err(_) if self.phase() == Phase::Stopping => return Ok(()),
            Err(error) => return Err(crate::agent_bridge::map_connection_error(error)),
        };
        // Before taking ownership, cancellation has no Agent store to drain.
        // Dropping the probe still tears down its transport/runtime first.
        let Some(selection) = selection else {
            return Ok(());
        };
        let permit = match selection {
            future_rpc::connection::AgentSelection::External(_) => {
                crate::future_platform::validate_external_environment()?;
                let mut state = self.0.state.lock().unwrap();
                if state.phase != Phase::Starting {
                    return Ok(());
                }
                state.phase = Phase::Ready;
                self.0.changed.notify_all();
                return Ok(());
            }
            future_rpc::connection::AgentSelection::LocalStart(permit) => {
                if mode == "external" {
                    return Err("External Agent is unavailable; attach-only mode never starts an embedded service".into());
                }
                permit
            }
        };
        {
            let state = self.0.state.lock().unwrap();
            if state.phase != Phase::Starting {
                return Ok(());
            }
        }
        AgentService::with_local_start(AgentConfig::default(), permit, |service| {
            {
                let mut state = self.0.state.lock().unwrap();
                if state.phase != Phase::Starting {
                    return Ok(());
                }
                state.owned = true;
            }
            service.run_with_cleanup(|| {
                // Only the lock owner may set first-launch environment defaults.
                crate::future_platform::apply_channel_environment_default()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let prepared = match service.prepare(cancel) {
                    Ok(prepared) => prepared,
                    Err(error) if error.is::<StartupCancelled>() => return Ok(()),
                    Err(error) => return Err(error),
                };
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("agent-runtime")
                    .worker_threads(2)
                    .max_blocking_threads(32)
                    .thread_stack_size(2 * 1024 * 1024)
                    .build()?;
                let result: anyhow::Result<()> = runtime.block_on(async {
                    let running = prepared.initialize().await?;
                    let handle = running.handle();
                    {
                        let mut state = self.0.state.lock().unwrap();
                        state.handle = Some(handle.clone());
                        if state.phase != Phase::Starting {
                            handle.request_shutdown(Duration::from_secs(5));
                        }
                    }
                    #[cfg(test)]
                    tests::before_first_serve_poll(self, &handle).await;
                    let (ready_tx, mut ready_rx) = tokio::sync::oneshot::channel();
                    let server = running.serve(Some(ready_tx));
                    tokio::pin!(server);
                    let mut awaiting_ready = true;
                    loop {
                        tokio::select! {
                            result = server.as_mut() => { result?; break; }
                            report = handle.wait_for_shutdown() => {
                                anyhow::ensure!(report.status == ShutdownStatus::Stopped,
                                    "Agent cleanup failed: {}", report.errors.join("; "));
                                server.as_mut().await?;
                                break;
                            }
                            ready = &mut ready_rx, if awaiting_ready => {
                                awaiting_ready = false;
                                // Cancellation can drop the readiness sender before
                                // serving begins. Keep polling both serving and drain;
                                // channel closure alone is not a cleanup failure.
                                if let Ok(identity) = ready {
                                    crate::host_environment::report(&format!("Agent identity version={} instance={}", identity.version, identity.agent_instance_id));
                                    let mut state = self.0.state.lock().unwrap();
                                    if state.phase == Phase::Starting {
                                        state.phase = Phase::Ready;
                                        crate::host_environment::report("embedded Agent ready");
                                    }
                                    self.0.changed.notify_all();
                                }
                            }
                        }
                    }
                    let report = handle.shutdown(Duration::from_secs(5)).await;
                    anyhow::ensure!(
                        report.status == ShutdownStatus::Stopped,
                        "Agent cleanup failed: {}",
                        report.errors.join("; ")
                    );
                    Ok(())
                });
                runtime.shutdown_timeout(Duration::from_secs(1));
                // Normalize only the typed startup cancellation, inside the
                // service cleanup scope. finish_scope must still verify drain
                // and preserve any real cleanup failure/retained instance lock.
                match result {
                    Err(error) if error.is::<StartupCancelled>() => Ok(()),
                    result => result,
                }
            })
        })
        .map_err(|error| error.to_string().into())
    }

    /// Never sends shutdown to external Agents. Failed stops are terminal and
    /// callers must refuse restart/reset/install until the process is exited.
    #[cfg(test)]
    pub(crate) fn stop(&self) -> Result<(), crate::AppError> {
        self.stop_until(Instant::now() + STOP_TIMEOUT)
    }
    pub(crate) fn stop_until(&self, overall: Instant) -> Result<(), crate::AppError> {
        let deadline = overall.min(Instant::now() + STOP_TIMEOUT);
        let mut state = self.0.state.lock().unwrap();
        if matches!(state.phase, Phase::Idle | Phase::Stopped) {
            return Ok(());
        }
        if state.phase == Phase::Ready && !state.owned && state.finished {
            state.phase = Phase::Stopped;
            return Ok(());
        }
        state.cancel.cancel();
        state.probe_cancel.notify_one();
        if !matches!(state.phase, Phase::Failed(_)) {
            state.phase = Phase::Stopping;
        }
        if let Some(handle) = &state.handle {
            handle.request_shutdown(Duration::from_secs(5));
        }
        self.0.changed.notify_all();
        while !state.finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.phase =
                    Phase::Failed("Agent cleanup timed out; restart requires process exit".into());
                break;
            }
            state = self.0.changed.wait_timeout(state, remaining).unwrap().0;
        }
        match &state.phase {
            Phase::Failed(error) => Err(error.clone().into()),
            _ => {
                state.phase = Phase::Stopped;
                Ok(())
            }
        }
    }

    pub(crate) fn request_stop(&self) {
        let mut state = self.0.state.lock().unwrap();
        if matches!(state.phase, Phase::Starting | Phase::Ready) {
            state.phase = Phase::Stopping;
        }
        state.cancel.cancel();
        state.probe_cancel.notify_one();
        if let Some(handle) = &state.handle {
            handle.request_shutdown(Duration::from_secs(5));
        }
        self.0.changed.notify_all();
    }
}
