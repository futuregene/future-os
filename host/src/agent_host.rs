//! Shared GUI/headless owner. The native worker holds the service scope and
//! its singleton lock through runtime teardown; clients retain only handles.
use future_agent::service::{
    AgentConfig, AgentHandle, AgentService, PreparedAgent, ShutdownStatus, StartupCancellation,
    StartupCancelled,
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
    cancel: StartupCancellation,
    probe_cancel: Arc<tokio::sync::Notify>,
    handle: Option<AgentHandle>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
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
                cancel: StartupCancellation::default(),
                probe_cancel: Arc::new(tokio::sync::Notify::new()),
                handle: None,
            }),
            changed: Condvar::new(),
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
        self.0.state.lock().unwrap().phase.clone()
    }
    pub fn owns_agent(&self) -> bool {
        self.0.state.lock().unwrap().owned
    }
    pub fn activity(&self) -> Result<Option<future_agent::service::AgentActivity>, crate::Error> {
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
                                    let driver = DRIVER
                                        .get()
                                        .ok_or("Entry must configure the Agent host driver")?;
                                    driver(&host, &configured)
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
            let state = self.0.state.lock().unwrap();
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
                result = future_rpc::connection::select_agent(
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
                        crate::environment::report(&format!("Agent identity version={} instance={}", identity.version, identity.agent_instance_id));
                        let mut state = self.0.state.lock().unwrap();
                        if state.phase == Phase::Starting {
                            state.phase = Phase::Ready;
                            crate::environment::report("embedded Agent ready");
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
    }

    /// Never sends shutdown to external Agents. Failed stops are terminal and
    /// callers must refuse restart/reset/install until the process is exited.
    #[cfg(any(test, feature = "test-support"))]
    pub fn stop(&self) -> Result<(), crate::Error> {
        self.stop_until(Instant::now() + STOP_TIMEOUT)
    }
    pub fn stop_until(&self, overall: Instant) -> Result<(), crate::Error> {
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

    pub fn request_stop(&self) {
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
mod test_driver;
