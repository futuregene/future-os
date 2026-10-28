//! Shared GUI/headless owner. The native worker holds the service scope and
//! its singleton lock through runtime teardown; clients retain only handles.
use future_agent::service::{
    AgentConfig, AgentHandle, AgentService, PreparedAgent, ServiceRetirement, ShutdownStatus,
    StartupCancellation, StartupCancelled,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
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
    /// Why the last worker yielded instead of owning anything: another process
    /// holds the instance lock. Kept beside `Stopped` (which is not a fault)
    /// so `start` and the status page report the holder's own refusal instead
    /// of a cancellation nobody asked for.
    yield_reason: Option<String>,
    cancel: StartupCancellation,
    probe_cancel: Arc<tokio::sync::Notify>,
    handle: Option<AgentHandle>,
    retirement: Option<ServiceRetirement>,
    recovery_requires_restart: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    recovery: Mutex<()>,
}

impl Shared {
    /// The host state as it is, poisoned or not.
    ///
    /// Poisoning says only that some thread panicked while holding this lock,
    /// not that the state is unusable: every field is either a plain value or
    /// a clone-able handle, and the values left behind still describe the
    /// phase the worker reached. Refusing them instead turns one broken worker
    /// into a host that can never be inspected, stopped or recovered again.
    /// Measured 2026-10-28: a single panicking thread poisoned the state, every
    /// later read panicked in turn, and the Desktop gave up on its own backend
    /// with "the backend service did not start" while the process kept running.
    ///
    /// Callers must therefore re-read the state rather than assume it, which
    /// they already do: it is shared with the worker thread.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
#[derive(Clone)]
pub struct AgentHost(Arc<Shared>);
impl Default for AgentHost {
    fn default() -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                phase: Phase::Idle,
                owned: false,
                finished: true,
                yield_reason: None,
                cancel: StartupCancellation::default(),
                probe_cancel: Arc::new(tokio::sync::Notify::new()),
                handle: None,
                retirement: None,
                recovery_requires_restart: false,
            }),
            changed: Condvar::new(),
            recovery: Mutex::new(()),
        }))
    }
}
/// Native entry driver. It creates/drives/drops runtimes on the host worker;
/// application policy and scoped Agent ownership stay in this library.
pub type AgentDriver = fn(&AgentHost, &str) -> Result<(), crate::Error>;
static DRIVER: std::sync::OnceLock<AgentDriver> = std::sync::OnceLock::new();
pub fn configure_driver(driver: AgentDriver) -> Result<(), crate::Error> {
    DRIVER
        .set(driver)
        .map_err(|_| "Agent host driver already configured".into())
}

/// Opaque discovery result; an entry cannot alter permissions or Agent ownership.
pub struct StartSelection {
    mode: String,
    cancel: StartupCancellation,
    result: Result<
        Option<future_rpc::connection::AgentSelection>,
        future_rpc::connection::ConnectionError,
    >,
}

static HOST: std::sync::LazyLock<AgentHost> = std::sync::LazyLock::new(AgentHost::default);
#[cfg(test)]
mod tests;
pub fn host() -> &'static AgentHost {
    &HOST
}

impl AgentHost {
    pub fn phase(&self) -> Phase {
        self.0.state().phase.clone()
    }
    pub fn owns_agent(&self) -> bool {
        self.0.state().owned
    }

    pub fn recovery_requires_restart(&self) -> bool {
        let state = self.0.state();
        state.recovery_requires_restart
            || (state.owned
                && state.finished
                && !state
                    .retirement
                    .as_ref()
                    .is_some_and(ServiceRetirement::is_complete))
    }

    /// Repair only after sustained loss of readiness. External services are
    /// detached, never stopped. Owned services must retire AND finish their
    /// native driver before their failure can be cleared. No command is replayed.
    pub fn recover(&self, configured: String) -> Result<(), crate::Error> {
        let Ok(_recovery) = self.0.recovery.try_lock() else {
            return Ok(());
        };
        if crate::request_scope::is_stopping() {
            return Err("Desktop is stopping".into());
        }
        {
            let state = self.0.state();
            if matches!(state.phase, Phase::Starting | Phase::Stopping) {
                return Ok(());
            }
            if !state.owned && !state.finished {
                return Ok(());
            }
            if state.recovery_requires_restart {
                return Err("Agent cleanup is incomplete; restart Desktop".into());
            }
        }
        let result = (|| {
            if self.owns_agent() {
                // This uses the service control handle, not the broken RPC
                // connection, and never freezes application-wide admission.
                let _ = self.stop_until(Instant::now() + STOP_TIMEOUT);
            }
            {
                let mut state = self.0.state();
                if !state.finished
                    || (state.owned
                        && !state
                            .retirement
                            .as_ref()
                            .is_some_and(ServiceRetirement::is_complete))
                {
                    state.recovery_requires_restart = true;
                    return Err("Agent cleanup is incomplete; restart Desktop".into());
                }
                if crate::request_scope::is_stopping() {
                    return Err("Desktop is stopping".into());
                }
                state.phase = Phase::Stopped;
                state.owned = false;
                state.handle = None;
                state.retirement = None;
                state.yield_reason = None;
            }
            self.start(configured)
        })();
        result
    }

    /// Give up an Agent this host only *attached* to, so a later [`Self::start`]
    /// probes and possibly launches again.
    ///
    /// Refused while this host owns the Agent: an owned service holds the
    /// instance lock until a full stop, and pretending otherwise would let a
    /// second process start one. Returns whether the host changed state.
    pub fn detach_attached(&self) -> bool {
        let mut state = self.0.state();
        if state.owned || state.phase != Phase::Ready {
            return false;
        }
        state.phase = Phase::Stopped;
        state.yield_reason = None;
        true
    }
    /// Used only after an update executor is ready, immediately before stop.
    /// External Agents cannot be reserved or stopped by this application.
    pub fn reserve_idle_update(&self) -> Result<bool, crate::Error> {
        let state = self.0.state();
        if !state.owned {
            return Ok(false);
        }
        state
            .handle
            .as_ref()
            .ok_or("Agent is still starting")?
            .close_idle_admission()
            .map_err(|error| error.to_string().into())
    }

    pub fn activity(&self) -> Result<Option<future_agent::service::AgentActivity>, crate::Error> {
        let state = self.0.state();
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

    pub fn start(&self, configured: String) -> Result<(), crate::Error> {
        // The launch's last step is the Agent coming up; the entry points arm a
        // watchdog over it, so both outcomes must close the launch out.
        match self.start_agent(configured) {
            Ok(()) => {
                crate::startup::global().ready();
                Ok(())
            }
            Err(error) => {
                crate::startup::global().failed();
                Err(error)
            }
        }
    }

    fn start_agent(&self, configured: String) -> Result<(), crate::Error> {
        if crate::request_scope::is_stopping() {
            return Err("Desktop is stopping".into());
        }
        {
            let mut state = self.0.state();
            match &state.phase {
                Phase::Ready => return Ok(()),
                Phase::Starting => {}
                Phase::Idle | Phase::Stopped => {
                    state.phase = Phase::Starting;
                    state.finished = false;
                    state.owned = false;
                    // The previous worker's yield belonged to its own attempt.
                    state.yield_reason = None;
                    state.handle = None;
                    state.retirement = None;
                    state.cancel = StartupCancellation::default();
                    state.probe_cancel = Arc::new(tokio::sync::Notify::new());
                    let host = self.clone();
                    if let Err(error) = std::thread::Builder::new()
                        .name("embedded-agent".into())
                        .spawn(move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let driver = DRIVER
                                        .get()
                                        .ok_or("Entry must configure the Agent host driver")?;
                                    driver(&host, &configured)
                                }));
                            let result = result
                                .unwrap_or_else(|_| Err("Embedded Agent worker panicked".into()));
                            let mut state = host.0.state();
                            if !matches!(state.phase, Phase::Failed(_)) {
                                // A busy instance lock is a yield, not a fault: the
                                // holder owns the Agent, so this process returns to
                                // `Stopped` and may try again once that owner
                                // releases it. Only a real failure is terminal. The
                                // holder's refusal is kept, because `Stopped` alone
                                // would otherwise read as "cancelled" to every caller.
                                let yield_reason = match &result {
                                    Err(crate::Error::AgentBusy(reason)) => Some(reason.clone()),
                                    _ => None,
                                };
                                let busy = yield_reason.is_some();
                                state.phase = match result {
                                    Ok(()) if !state.owned && state.phase == Phase::Ready => {
                                        Phase::Ready
                                    }
                                    Ok(()) => Phase::Stopped,
                                    Err(_) if busy => Phase::Stopped,
                                    Err(error) => Phase::Failed(error.to_string()),
                                };
                                state.yield_reason = yield_reason;
                            }
                            state.finished = true;
                            crate::environment::report(&format!(
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
        let deadline = Instant::now() + start_timeout();
        let mut state = self.0.state();
        loop {
            match &state.phase {
                Phase::Ready => return Ok(()),
                Phase::Starting => {}
                Phase::Failed(error) => return Err(error.clone().into()),
                // A held instance lock ends the worker in `Stopped` with the
                // holder's own refusal recorded: report that, so the caller can
                // wait for the owner rather than believe its start was cancelled.
                Phase::Stopped => match state.yield_reason.clone() {
                    Some(reason) => return Err(crate::Error::AgentBusy(reason)),
                    None => return Err("Agent startup was cancelled".into()),
                },
                _ => return Err("Agent startup was cancelled".into()),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.cancel.cancel();
                state.probe_cancel.notify_one();
                state.phase = Phase::Failed("Agent startup timed out; restart Desktop".into());
                return Err("Agent startup timed out; restart Desktop".into());
            }
            state = self
                .0
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// Select an external Agent or obtain its existing local-start permission.
    /// The entry drives this future and drops the probe runtime before handing
    /// the opaque result back to `run_selected`.
    pub async fn select(&self, configured: &str) -> Result<StartSelection, crate::Error> {
        crate::install_rustls_provider();
        let mode = std::env::var("FUTURE_DESKTOP_AGENT_MODE").unwrap_or_else(|_| "auto".into());
        if !matches!(mode.as_str(), "auto" | "external") {
            return Err("FUTURE_DESKTOP_AGENT_MODE must be auto or external".into());
        }
        let (cancel, probe_cancel, starting) = {
            let state = self.0.state();
            (
                state.cancel.clone(),
                state.probe_cancel.clone(),
                state.phase == Phase::Starting,
            )
        };
        let result = if starting {
            tokio::select! {
                biased;
                _ = probe_cancel.notified() => Ok(None),
                // Retried, not a single attempt: this decision is what turns a
                // momentarily stalled Agent into a permanent "local startup is
                // forbidden" refusal, and the same transient answer that made one
                // probe miss (a descheduled process, a socket stalled behind a
                // storage write) is gone on the next fresh deadline.
                result = future_rpc::supervision::select_agent_with_retries(
                    configured,
                    crate::build_info::VERSION,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                ) => result.map(Some),
            }
        } else {
            Ok(None)
        };
        Ok(StartSelection {
            mode,
            cancel,
            result,
        })
    }

    /// Keep the Agent service and its singleton lease around the complete entry
    /// driver lifetime, including runtime teardown. Fields of the selection and
    /// service state remain private; the entry only supplies runtime execution.
    pub fn run_selected(
        &self,
        selected: StartSelection,
        drive: impl FnOnce(PreparedAgent<'_, '_>) -> anyhow::Result<()>,
    ) -> Result<(), crate::Error> {
        let StartSelection {
            mode,
            cancel,
            result,
        } = selected;
        let selection = match result {
            Ok(selection) => selection,
            Err(_) if self.phase() == Phase::Stopping => return Ok(()),
            Err(error) => return Err(crate::agent_client::map_connection_error(error)),
        };
        // Before taking ownership, cancellation has no Agent store to drain.
        // Dropping the probe still tears down its transport/runtime first.
        let Some(selection) = selection else {
            return Ok(());
        };
        let permit = match selection {
            future_rpc::connection::AgentSelection::External(_) => {
                crate::future_platform::validate_external_environment()?;
                let mut state = self.0.state();
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
            let state = self.0.state();
            if state.phase != Phase::Starting {
                return Ok(());
            }
        }
        let config = AgentConfig::default();
        AgentService::with_local_start(config, permit, |service| {
            {
                let mut state = self.0.state();
                if state.phase != Phase::Starting {
                    return Ok(());
                }
                state.owned = true;
                state.retirement = Some(service.retirement());
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
                let result = drive(prepared);
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

    /// Run the prepared service on the runtime owned by the entry. Shutdown
    /// reports and readiness keep the original ordering and bounded drain.
    pub async fn serve(&self, prepared: PreparedAgent<'_, '_>) -> anyhow::Result<()> {
        let running = prepared.initialize().await?;
        let handle = running.handle();
        {
            let mut state = self.0.state();
            state.handle = Some(handle.clone());
            if state.phase != Phase::Starting {
                handle.request_shutdown(AGENT_STOP_GRACE);
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
                        // Publish readiness *before* either report: `report`
                        // writes to stderr and the host log, so a blocked log
                        // handle must never sit between a serving Agent and the
                        // Ready phase the launch is waiting on. Reordering these
                        // back (log first, phase second) reintroduces exactly
                        // that: the launch stays in `Starting` while the log
                        // write hangs, and nothing downstream can tell why.
                        let became_ready = {
                            let mut state = self.0.state();
                            let became_ready = state.phase == Phase::Starting;
                            if became_ready {
                                state.phase = Phase::Ready;
                            }
                            became_ready
                        };
                        self.0.changed.notify_all();
                        crate::environment::report(&format!("Agent identity version={} instance={}", identity.version, identity.agent_instance_id));
                        if became_ready {
                            crate::environment::report("embedded Agent ready");
                        }
                    }
                }
            }
        }
        let report = handle.shutdown(AGENT_STOP_GRACE).await;
        anyhow::ensure!(
            report.status == ShutdownStatus::Stopped,
            "Agent cleanup failed: {}",
            report.errors.join("; ")
        );
        Ok(())
    }

    /// Never sends shutdown to external Agents. Failed stops are terminal and
    /// callers must refuse restart/reset/install until the process is exited.
    #[cfg(any(test, feature = "test-support"))]
    pub fn stop(&self) -> Result<(), crate::Error> {
        self.stop_until(Instant::now() + STOP_TIMEOUT)
    }
    pub fn stop_until(&self, overall: Instant) -> Result<(), crate::Error> {
        let now = Instant::now();
        let deadline = overall.min(now + STOP_TIMEOUT);
        // Both windows come from `deadline`: the wait below observes it, and the
        // Agent's own drain is given what is left of it. They therefore expire
        // together — the host can no longer call a stop failed while the Agent is
        // still inside the window it was handed.
        let budget = deadline.saturating_duration_since(now);
        let window = agent_stop_window(budget);
        let mut state = self.0.state();
        if matches!(state.phase, Phase::Idle | Phase::Stopped) {
            state.yield_reason = None;
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
            handle.request_shutdown(window);
        }
        self.0.changed.notify_all();
        while !state.finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.phase = Phase::Failed(stop_timeout_message(budget));
                break;
            }
            state = self
                .0
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        match &state.phase {
            Phase::Failed(error) => Err(error.clone().into()),
            _ => {
                state.phase = Phase::Stopped;
                state.yield_reason = None;
                Ok(())
            }
        }
    }

    /// The last worker's yield reason, if the previous start stopped because
    /// another process owns this data root's Agent. `None` for every other
    /// terminal phase, so a caller never reads a stale refusal.
    pub fn yield_reason(&self) -> Option<String> {
        self.0.state().yield_reason.clone()
    }

    pub fn request_stop(&self) {
        let mut state = self.0.state();
        if matches!(state.phase, Phase::Starting | Phase::Ready) {
            state.phase = Phase::Stopping;
        }
        state.cancel.cancel();
        state.probe_cancel.notify_one();
        if let Some(handle) = &state.handle {
            handle.request_shutdown(AGENT_STOP_GRACE);
        }
        self.0.changed.notify_all();
    }
}

/// The window one stop hands the Agent for its own drain.
///
/// A stop has exactly one deadline, and this is the Agent's share of it: what is
/// left of the caller's budget, capped by the fixed grace an ordinary stop has
/// always given. Deriving it from that deadline instead of using the constant
/// directly is what keeps the two in step — a fixed 5 s window next to a shorter
/// wait meant the host could publish `Failed` ("restart requires process exit")
/// while the Agent still had seconds of the time it had been given, and the stop
/// then read as unrecoverable for no reason other than the two numbers
/// disagreeing.
pub const AGENT_STOP_GRACE: Duration = Duration::from_secs(5);

/// What the Agent is told for one stop: never more than what is left of the
/// budget, never more than the grace. An exhausted budget therefore hands out no
/// window at all — the Agent's own verdict then lands with the host's instead of
/// being cut off — rather than becoming an unbounded wait.
fn agent_stop_window(remaining: Duration) -> Duration {
    remaining.min(AGENT_STOP_GRACE)
}

/// The one wording of a stop's verdict. An ordinary timeout names the Agent's
/// cleanup, which is what ran out; a budget that was already spent when the stop
/// began says so instead, because nothing of the Agent's own had a chance to run
/// and "cleanup timed out" would send the operator looking for a stuck drain.
fn stop_timeout_message(budget: Duration) -> String {
    if budget.is_zero() {
        "Agent cleanup timed out before it began (the shutdown budget was already spent); \
         restart requires process exit"
            .into()
    } else {
        "Agent cleanup timed out; restart requires process exit".into()
    }
}

/// How long `start` waits before it cancels a worker that has not published a
/// phase. Production always uses `START_TIMEOUT`; the re-exec fixtures can shorten
/// it through their own case environment, which is the only way to observe the
/// deadline arm without holding a start for three minutes. The override is
/// compiled out of every non-test build.
fn start_timeout() -> Duration {
    #[cfg(test)]
    if let Ok(millis) = std::env::var("FUTURE_AGENT_HOST_TEST_START_MS") {
        if let Ok(millis) = millis.parse::<u64>() {
            return Duration::from_millis(millis);
        }
    }
    START_TIMEOUT
}

#[cfg(test)]
mod block_fixtures {
    //! The `agent_host` residual block, driven through the same subprocess
    //! re-exec `agent_host::tests` uses: each case owns its process, `FUTURE_HOME`
    //! and agent socket, so a one-way phase, a blocked startup or a stop deadline
    //! cannot leak into the shared libtest process.
    use super::*;
    use std::path::Path;
    use std::process::{Command, Stdio};

    const CASE: &str = "FUTURE_AGENT_HOST_BLOCK_CASE";

    #[test]
    fn a_stopping_desktop_refuses_a_start_before_any_worker_exists() {
        run_case("stopping");
    }

    #[test]
    fn a_ready_host_accepts_a_repeated_start_without_a_second_worker() {
        run_case("ready-repeat");
    }

    #[test]
    fn a_start_that_never_becomes_ready_fails_at_the_startup_deadline() {
        run_case("timeout");
    }

    #[cfg(unix)]
    #[test]
    fn a_worker_that_cannot_finish_fails_the_stop_at_the_cleanup_deadline() {
        run_case("stop-deadline");
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_during_the_external_environment_check_never_publishes_ready() {
        run_case("external-stop");
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_before_prepare_is_reported_as_a_clean_cancellation() {
        run_case("cancel-prepare");
    }

    #[cfg(unix)]
    #[test]
    fn a_prepare_that_sees_a_moved_home_is_a_real_failure() {
        run_case("moved-home");
    }

    fn run_case(case: &str) {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let home = tempfile::tempdir_in(&parent).unwrap();
        // The socket path must stay short: a unix `sun_path` is 104 bytes on
        // macOS, so `target/host-fixtures/...` would exceed it and the connect
        // would fail with ENAMETOOLONG instead of the missing-endpoint error.
        let socket = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "agent_host::block_fixtures::host_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CASE, case)
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("FUTURE_HOME", home.path().join(".future"))
            .env("FUTURE_DESKTOP_AGENT_MODE", "auto")
            .env("FUTURE_AGENT_SOCKET", socket.path().join("agent.sock"))
            .env_remove("FUTURE_AGENT_GRPC_ADDR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if case == "timeout" {
            command.env("FUTURE_AGENT_HOST_TEST_START_MS", "300");
        }
        let mut child = command.spawn().unwrap();
        // The cleanup-deadline case spends `STOP_TIMEOUT` waiting on purpose.
        let deadline = Instant::now() + Duration::from_secs(120);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "{case} timed out: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        // Hoisted: the message arguments of a passing assertion are evaluated
        // on failure alone, so an inline `from_utf8_lossy` here would be a
        // residual line in every run.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{case}: {stdout} {stderr}");
    }

    /// A cancelled start returns as soon as it observes the new phase, which can
    /// precede the worker publishing its terminal one; wait for the worker too.
    #[cfg(unix)]
    fn settled(host: &AgentHost, want: impl Fn(&Phase) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut phase = host.phase();
        while Instant::now() < deadline && !want(&phase) {
            std::thread::sleep(Duration::from_millis(10));
            phase = host.phase();
        }
        want(&phase)
    }

    /// Waits until the worker has the fixture file open for reading, then returns
    /// the writer that unblocks it. `ENXIO` means no reader is open yet.
    #[cfg(unix)]
    fn blocked_reader(path: &Path) -> std::fs::File {
        use std::os::unix::fs::OpenOptionsExt;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)
            {
                Ok(writer) => return writer,
                Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                    let target = path.display();
                    assert!(
                        Instant::now() < deadline,
                        "the fixture never reached its read: {target}"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("open fixture {}: {error}", path.display()),
            }
        }
    }

    #[cfg(unix)]
    fn fifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn host_fixture() {
        let Ok(case) = std::env::var(CASE) else {
            return;
        };
        test_driver::install();
        let host = AgentHost::default();
        if case == "stopping" {
            // Closing producer admission is synchronous: `start` must observe
            // it before scheduling a worker. Desktop tests cover coordinator wiring.
            crate::request_scope::close_admission();
            let error = host.start("auto".into()).unwrap_err();
            assert!(error.to_string().contains("Desktop is stopping"), "{error}");
            assert_eq!(host.phase(), Phase::Idle);
            assert!(!host.owns_agent());
            return;
        }
        if case == "ready-repeat" {
            let _mock = crate::test_support::agent::mock_agent();
            host.start(crate::agent_client::raw_agent_addr()).unwrap();
            assert_eq!(host.phase(), Phase::Ready);
            assert!(
                !host.owns_agent(),
                "an attached Agent stays owned by its own process"
            );
            host.start("auto".into()).unwrap();
            assert_eq!(host.phase(), Phase::Ready);
            host.stop().unwrap();
            assert_eq!(host.phase(), Phase::Stopped);
            return;
        }
        if case == "timeout" {
            let endpoint = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            endpoint.set_nonblocking(true).unwrap();
            let address = endpoint.local_addr().unwrap();
            let starter = host.clone();
            let thread = std::thread::spawn(move || starter.start(format!("http://{address}")));
            // A peer that accepts but never answers keeps the probe in flight,
            // which is the state the startup deadline has to cut short.
            let deadline = Instant::now() + Duration::from_secs(5);
            let _peer = loop {
                match endpoint.accept() {
                    Ok((peer, _)) => break peer,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "the probe did not connect");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept probe: {error}"),
                }
            };
            let error = thread.join().unwrap().unwrap_err().to_string();
            assert!(error.contains("startup timed out"), "{error}");
            let phase = host.phase();
            assert!(
                matches!(phase, Phase::Failed(_)),
                "a timed-out start never publishes Ready: {phase:?}"
            );
            let cleanup = host.stop().unwrap_err().to_string();
            assert!(cleanup.contains("startup timed out"), "{cleanup}");
            return;
        }
        #[cfg(unix)]
        if case == "stop-deadline" {
            // Block the storage import: the worker then cannot reach `finished`
            // before the stop deadline, which is the arm under test.
            let sessions = root_sessions();
            std::fs::write(
                sessions.join("a.jsonl"),
                serde_json::json!({"id":"e1","type":"user","role":"user","content":[{"type":"text","text":"fixture"}],"timestamp":"2026-01-01T00:00:00Z"}).to_string() + "\n",
            )
            .unwrap();
            let blocked = sessions.join("b.jsonl");
            fifo(&blocked);
            let starter = host.clone();
            let thread = std::thread::spawn(move || starter.start("auto".into()));
            let mut writer = blocked_reader(&blocked);
            // Opening the writer unblocks the reader's `File::open`; its next
            // cancellation checkpoint runs *before* the read that then blocks.
            // Let it settle into the read, or the cancel below would abort the
            // import at that checkpoint instead of leaving it stuck.
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                host.phase(),
                Phase::Starting,
                "the import must still be in flight, not finished"
            );
            let started = Instant::now();
            let error = host.stop();
            let error = error.unwrap_err().to_string();
            assert!(error.contains("cleanup timed out"), "{error}");
            let elapsed = started.elapsed();
            assert!(
                elapsed >= STOP_TIMEOUT,
                "the deadline is what fails the stop: {elapsed:?}"
            );
            assert!(matches!(host.phase(), Phase::Failed(_)));
            // Release the worker so the process exits with no stuck import.
            use std::io::Write;
            let _ = writer.write_all(b"\n");
            drop(writer);
            let _ = thread.join();
            return;
        }
        #[cfg(unix)]
        if case == "external-stop" {
            let _mock = crate::test_support::agent::mock_agent();
            let agent_dir = future_rpc::home::future_home().join("agent");
            std::fs::create_dir_all(&agent_dir).unwrap();
            let auth = agent_dir.join("auth.json");
            fifo(&auth);
            let starter = host.clone();
            let address = crate::agent_client::raw_agent_addr();
            let thread = std::thread::spawn(move || starter.start(address));
            // The reader proves the worker is inside the external-owner policy
            // check, i.e. after the probe resolved and before it takes the lock.
            let mut writer = blocked_reader(&auth);
            host.request_stop();
            use std::io::Write;
            writer.write_all(b"{}").unwrap();
            drop(writer);
            let error = thread.join().unwrap().unwrap_err().to_string();
            assert!(
                error.contains("startup was cancelled"),
                "a stop in that window is a cancellation, not a ready: {error}"
            );
            let stopped = settled(&host, |phase| *phase == Phase::Stopped);
            let phase = host.phase();
            assert!(
                stopped,
                "the cancelled attach publishes a terminal phase: {phase:?}"
            );
            assert!(!host.owns_agent());
            return;
        }
        #[cfg(unix)]
        if case == "cancel-prepare" {
            let agent_dir = future_rpc::home::future_home().join("agent");
            std::fs::create_dir_all(&agent_dir).unwrap();
            let auth = agent_dir.join("auth.json");
            fifo(&auth);
            let starter = host.clone();
            let thread = std::thread::spawn(move || starter.start("auto".into()));
            let mut writer = blocked_reader(&auth);
            // Cancellation lands before `prepare`, whose first checkpoint then
            // reports a typed startup cancellation rather than a failure.
            host.request_stop();
            use std::io::Write;
            writer
                .write_all(br#"{"future":{"base_url":"https://fixture.invalid/api"}}"#)
                .unwrap();
            drop(writer);
            let error = thread.join().unwrap().unwrap_err().to_string();
            assert!(error.contains("startup was cancelled"), "{error}");
            let stopped = settled(&host, |phase| *phase == Phase::Stopped);
            let phase = host.phase();
            assert!(
                stopped,
                "a cancelled prepare publishes a terminal phase: {phase:?}"
            );
            return;
        }
        #[cfg(unix)]
        if case == "moved-home" {
            let agent_dir = future_rpc::home::future_home().join("agent");
            std::fs::create_dir_all(&agent_dir).unwrap();
            let auth = agent_dir.join("auth.json");
            fifo(&auth);
            let starter = host.clone();
            let thread = std::thread::spawn(move || starter.start("auto".into()));
            let mut writer = blocked_reader(&auth);
            // A root that moved after the configuration was captured is refused
            // by the service rather than silently initialized elsewhere.
            let moved = std::env::temp_dir().join("future-agent-host-moved-home");
            std::env::set_var("FUTURE_HOME", &moved);
            use std::io::Write;
            writer
                .write_all(br#"{"future":{"base_url":"https://fixture.invalid/api"}}"#)
                .unwrap();
            drop(writer);
            let error = thread.join().unwrap().unwrap_err().to_string();
            assert!(error.contains("home changed"), "{error}");
            assert!(matches!(host.phase(), Phase::Failed(_)));
            assert!(!moved.join("agent").exists());
            return;
        }
        panic!("unknown case {case}");
    }

    #[cfg(unix)]
    fn root_sessions() -> std::path::PathBuf {
        let sessions = future_rpc::home::future_home().join("agent/sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        sessions
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// A host that owns nothing reports no activity: the GUI's quit decision
    /// falls back to *its own* submitted runs instead of asking an external
    /// Agent for work it does not own.
    #[test]
    fn a_fresh_host_is_idle_unowned_and_reports_no_activity() {
        let host = AgentHost::default();
        assert_eq!(host.phase(), Phase::Idle);
        assert!(!host.owns_agent());
        assert!(
            host.activity().unwrap().is_none(),
            "an unowned host must not claim an Agent's activity"
        );
        // A stop request against an idle host is a no-op: it must not panic,
        // claim ownership or move the phase.
        host.request_stop();
        assert_eq!(host.phase(), Phase::Idle);
        assert!(!host.owns_agent());
        assert!(host.activity().unwrap().is_none());
    }
}

#[cfg(test)]
mod stop_budget_tests {
    use super::*;

    /// One deadline, two windows. An ordinary stop is unchanged (the Agent keeps
    /// exactly the 5 s grace it had), a tighter budget is passed through whole,
    /// and a spent budget is no window at all — never an unbounded wait.
    #[test]
    fn the_agent_window_is_the_budgets_remainder_capped_by_the_grace() {
        assert_eq!(AGENT_STOP_GRACE, Duration::from_secs(5));
        // `stop()` is `stop_until(now + STOP_TIMEOUT)`: the grace wins, unchanged.
        assert_eq!(agent_stop_window(STOP_TIMEOUT), AGENT_STOP_GRACE);
        // `ServiceStop` passes the process-wide shutdown deadline: same answer.
        assert_eq!(agent_stop_window(Duration::from_secs(20)), AGENT_STOP_GRACE);
        // A budget shorter than the grace is handed over whole, never padded.
        for remaining in [
            Duration::from_secs(4),
            Duration::from_millis(300),
            Duration::from_millis(1),
        ] {
            assert_eq!(agent_stop_window(remaining), remaining);
        }
        // Spent: no window at all, so the Agent's own verdict lands with the
        // host's instead of after it.
        assert_eq!(agent_stop_window(Duration::ZERO), Duration::ZERO);
        // The wait is the budget itself, so the window can never outlive it.
        for budget in [
            Duration::ZERO,
            Duration::from_millis(1),
            Duration::from_secs(3),
            AGENT_STOP_GRACE,
            Duration::from_secs(30),
        ] {
            assert!(
                agent_stop_window(budget) <= budget.min(AGENT_STOP_GRACE),
                "{budget:?}"
            );
        }
    }

    /// Two wordings for one verdict: "cleanup timed out" points the operator at a
    /// stuck Agent, which is the wrong lead when nothing of the Agent's own ran.
    #[test]
    fn an_already_spent_budget_is_not_reported_as_the_agents_own_timeout() {
        assert_eq!(
            stop_timeout_message(Duration::from_secs(10)),
            "Agent cleanup timed out; restart requires process exit"
        );
        let spent = stop_timeout_message(Duration::ZERO);
        assert!(spent.contains("before it began"), "{spent}");
        assert!(
            spent.contains("restart requires process exit"),
            "the terminal consequence is unchanged: {spent}"
        );
    }
}

#[cfg(test)]
mod test_driver;
