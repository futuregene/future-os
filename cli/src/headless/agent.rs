//! Headless uses the same scoped in-process owner as GUI. The bundled CLI
//! remains needed only for short maintenance commands such as skill bootstrap.

use std::time::{Duration, Instant};

use future_rpc::supervision::{
    probe_local_agent, Attached, LocalAgentState, ReadinessLoss, RestartBudget, WAIT_FOR_OWNER,
    WAIT_NOTICE_AFTER,
};

/// How long a probe may take before it is not worth waiting for inside the
/// host's one-second loop. It is the same budget the client layer uses.
const PROBE_CONNECT: Duration = Duration::from_millis(400);
const PROBE_HANDSHAKE: Duration = Duration::from_secs(2);

/// How long an attached Agent may stay unseen before the supervisor acts.
/// One missed probe is a restart in progress, not a death.
const UNSEEN_TOLERANCE: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct Agent {
    budget: RestartBudget,
    loss: ReadinessLoss,
    unseen_since: Option<Instant>,
    waiting_for: Option<(Instant, String)>,
    /// When the Agent last became reachable again. The restart budget is only
    /// cleared once it stays reachable (`STABLE_AFTER`), so an Agent the host
    /// keeps killing cannot reset the breaker on every lap.
    reachable_since: Option<Instant>,
}
impl Agent {
    pub(super) async fn ensure_running(&mut self) -> Result<(), future_host::Error> {
        let address = future_host::agent_client::raw_agent_addr();
        tokio::task::spawn_blocking(move || host().start(address))
            .await
            .map_err(|error| error.to_string())??;
        Ok(())
    }
    pub(super) fn check_running(&mut self) -> Result<(), future_host::Error> {
        match host().phase() {
            future_host::agent_host::Phase::Failed(error) if host().recovery_requires_restart() => {
                Err(error.into())
            }
            _ => Ok(()),
        }
    }

    /// Notice that the Agent this host is serving died and bring one back.
    ///
    /// This is what [`Self::check_running`] cannot see: it only reads this
    /// host's own phase, which stays `Ready` forever while the Agent lives in
    /// *another* process. Probing the endpoint is the only honest signal, and
    /// when nobody answers while this host owns nothing, the right move is to
    /// start an Agent (or, if another process holds the instance lock, to wait
    /// for it instead of starting a second one).
    ///
    /// Bounded on both sides: waiting for another owner is capped by
    /// [`WAIT_FOR_OWNER`], and this process's own restarts by [`RestartBudget`],
    /// so an Agent the host keeps killing cannot become a restart loop.
    pub(super) async fn supervise(&mut self) -> Result<(), future_host::Error> {
        let address = future_host::agent_client::raw_agent_addr();
        let state = probe_local_agent(&address, PROBE_CONNECT, PROBE_HANDSHAKE).await;
        match state {
            LocalAgentState::Attached => {
                self.loss.reset();
                self.unseen_since = None;
                self.waiting_for = None;
                let now = Instant::now();
                let since = *self.reachable_since.get_or_insert(now);
                if Attached::of(since, now) == Attached::Stable {
                    self.budget.reset();
                }
                Ok(())
            }
            LocalAgentState::Busy(detail) | LocalAgentState::Unresponsive(detail) => {
                self.reachable_since = None;
                if host().owns_agent() {
                    let now = Instant::now();
                    if self.loss.confirmed(now) && self.budget.allow(now) {
                        self.loss.reset();
                        return self.repair(address).await;
                    }
                    return Ok(());
                }
                // Another process is starting (or serving on an endpoint this
                // client cannot see). Wait — never start a second Agent.
                self.unseen_since = None;
                let now = Instant::now();
                match &self.waiting_for {
                    Some((since, _)) if now.duration_since(*since) < WAIT_FOR_OWNER => {
                        let waited = now.duration_since(*since);
                        if waited >= WAIT_NOTICE_AFTER && self.should_notice(waited) {
                            eprintln!(
                                "FutureOS: waiting for the Agent another process is starting \
                                 ({waited:?} of {WAIT_FOR_OWNER:?}). {detail}"
                            );
                        }
                        Ok(())
                    }
                    Some((since, _)) => Err(format!(
                        "another process has held this data root's Agent lock for {WAIT_FOR_OWNER:?} \
                         without serving; it is not starting. Stop it and start this entry again. \
                         (waiting since {since:?})"
                    )
                    .into()),
                    None => {
                        self.waiting_for = Some((now, detail.clone()));
                        eprintln!("FutureOS: another process owns the Agent; waiting for it instead of starting a second one. {detail}");
                        Ok(())
                    }
                }
            }
            LocalAgentState::Startable => {
                self.waiting_for = None;
                self.reachable_since = None;
                // Grace: one missed probe is a restart, not a death.
                let now = Instant::now();
                let since = *self.unseen_since.get_or_insert(now);
                if now.duration_since(since) < UNSEEN_TOLERANCE {
                    return Ok(());
                }
                if !self.budget.allow(now) {
                    return Ok(());
                }
                self.unseen_since = None;
                eprintln!("FutureOS: the Agent is gone; starting a new one.");
                self.reachable_since = None;
                // A host that only attached to the dead Agent must give it up
                // first, or `start` reports the stale `Ready` and does nothing.
                self.repair(address).await
            }
            LocalAgentState::Blocked(reason) => {
                // Waiting cannot repair this (version/protocol mismatch, an
                // explicit endpoint that is unreachable, a stop in progress).
                Err(reason.into())
            }
        }
    }

    async fn repair(&mut self, address: String) -> Result<(), future_host::Error> {
        let result = tokio::task::spawn_blocking(move || host().recover(address))
            .await
            .map_err(|error| error.to_string())?;
        if let Err(error) = &result {
            future_host::environment::report(&format!("Agent recovery: {error}"));
        }
        // A failed cleanup must exit so the process manager can safely replace
        // this process. Transient startup failures remain under supervision.
        if host().recovery_requires_restart() {
            result
        } else {
            Ok(())
        }
    }

    /// Rate-limit the long-wait notice to once a minute.
    fn should_notice(&self, waited: Duration) -> bool {
        waited.as_secs() % 60 < 1
    }

    /// `reason` is the shared stop operation's reason, so a cleanup that runs out
    /// of time names what ended the entry instead of a generic exit.
    pub(super) async fn shutdown(
        &mut self,
        reason: &'static str,
    ) -> Result<(), future_host::Error> {
        tokio::task::spawn_blocking(move || super::stop(reason))
            .await
            .map_err(|error| error.to_string())?
    }
}

fn host() -> &'static future_host::agent_host::AgentHost {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        future_host::agent_host::configure_driver(drive)
            .expect("entry installs its Agent driver once");
    });
    future_host::agent_host::host()
}

fn drive(
    host: &future_host::agent_host::AgentHost,
    configured: &str,
) -> Result<(), future_host::Error> {
    let probe = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let selected = probe.block_on(host.select(configured));
    drop(probe);
    host.run_selected(selected?, |prepared| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("agent-runtime")
            .worker_threads(2)
            .max_blocking_threads(32)
            .thread_stack_size(2 * 1024 * 1024)
            .build()?;
        let result = runtime.block_on(host.serve(prepared));
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_without_an_owned_service_is_a_noop() {
        let _home = future_host::auth_store::test_support::HomeGuard::new("headless-stop");
        let _mock = future_host::test_support::agent::mock_agent();
        let mut agent = Agent::default();
        agent.check_running().unwrap();
        agent.shutdown("headless_exit").await.unwrap();
        assert!(!super::host().owns_agent());
    }
}

/// `check_running` and the busy-lock yield, driven in a **child process**.
///
/// The host is a process-global (`agent_host::HOST`) and a failed start leaves it
/// `Failed` with no way back: nothing in the crate resets it, because production
/// never needs to. Leaving it Failed here would make every later test that starts
/// or inspects the host see a poisoned global, so the cases re-exec this same
/// libtest binary with their own `HOME`/`FUTURE_HOME`/socket — the shape
/// `agent_host::block_fixtures` and `agent_supervisor::block_fixtures` use.
#[cfg(all(test, unix))]
mod block_fixtures {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CASE: &str = "FUTURE_HEADLESS_AGENT_CASE";
    /// Written by the child once the case's assertions have passed, and required
    /// by the parent: the pair is what makes "the case ran" observable.
    const CASE_RAN: &str = "HEADLESS-AGENT-CASE-RAN";
    /// An endpoint nothing listens on: a refusal no wait repairs, which is how a
    /// genuine `Failed` is reached through public behaviour.
    const DEAD_ENDPOINT: &str = "http://127.0.0.1:1";

    /// A host left `Failed` by a refused start is reported with **that** error,
    /// not silently treated as running — the difference between a server that
    /// says why it cannot serve and one that serves a broken bridge.
    #[test]
    fn a_failed_discovery_keeps_supervision_available() {
        run_case("failed");
    }

    /// A held instance lock is **not** a failure: the other process owns the
    /// Agent. The host must stay restartable (so supervision can take over when
    /// that owner exits) and `check_running` must not report a fault, or headless
    /// would exit instead of waiting.
    #[test]
    fn a_busy_lock_leaves_the_host_restartable() {
        run_case("busy");
    }

    fn run_case(case: &str) {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let home = tempfile::tempdir_in(&parent).unwrap();
        // A unix `sun_path` is 104 bytes on macOS, so the socket lives in the
        // short system temp dir rather than beside the fixture home.
        let socket = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "headless::agent::block_fixtures::host_fixture",
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
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                // One line on purpose: a message that spans lines leaves its
                // continuation lines uncovered in this very file, and a `Debug`
                // `Output` carries both streams.
                panic!("{case} timed out: {output:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        // Hoisted: a passing `assert!` never evaluates its message arguments, and
        // this file is inside the measured scope.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // The child prints this only after the arm's own assertions passed. Without
        // it, "the child exited 0" would also be true for a filter that selected no
        // test at all — an exit status is not evidence that a case ran.
        let ran = stderr.contains(CASE_RAN);
        assert!(output.status.success() && ran, "{case}: {stdout} {stderr}");
    }

    #[test]
    fn host_fixture() {
        let Ok(case) = std::env::var(CASE) else {
            return;
        };
        let root = future_rpc::home::future_home();
        match case.as_str() {
            // A dead explicit endpoint: `select_agent` refuses to fall back to a
            // local instance, so the host reaches `Failed` through its public
            // behaviour rather than through a test-only setter.
            "failed" => {
                let refused = super::host()
                    .start(DEAD_ENDPOINT.into())
                    .err()
                    .map(|error| error.to_string());
                let failed = matches!(
                    super::host().phase(),
                    future_host::agent_host::Phase::Failed(_)
                );
                let mut agent = Agent::default();
                let recoverable = agent.check_running().is_ok();
                assert!(refused.is_some(), "a dead endpoint must refuse the start");
                assert!(failed, "the refused start must leave the host Failed");
                assert!(
                    recoverable,
                    "failed discovery owns no service requiring process exit"
                );
                assert!(!super::host().recovery_requires_restart());
            }
            // A held instance lock: another process is starting (or serving) the
            // Agent. The start is refused, but the host stays `Stopped` so a later
            // `start` can take over, and `check_running` reports no fault.
            "busy" => {
                std::fs::create_dir_all(root.join("agent")).unwrap();
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(root.join("agent/agent-instance.lock"))
                    .unwrap();
                fs2::FileExt::try_lock_exclusive(&file).unwrap();
                let refused = super::host().start("auto".into()).err().is_some();
                let restartable = super::host().phase() == future_host::agent_host::Phase::Stopped;
                let mut agent = Agent::default();
                let quiet = agent.check_running().is_ok();
                assert!(refused, "a held lock must refuse the start on this path");
                assert!(
                    restartable,
                    "a busy lock must leave the host restartable, not Failed"
                );
                assert!(quiet, "a busy lock is not a fault check_running reports");
            }
            other => panic!("unknown case {other}"),
        }
        eprintln!("{CASE_RAN} {case}");
    }
}
