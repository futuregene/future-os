//! Headless uses the same scoped in-process owner as GUI. The bundled CLI
//! remains needed only for short maintenance commands such as skill bootstrap.
#[derive(Default)]
pub(super) struct Agent;
impl Agent {
    pub(super) async fn ensure_running(&mut self) -> Result<(), crate::AppError> {
        let address = crate::agent_bridge::raw_agent_addr();
        tokio::task::spawn_blocking(move || crate::agent_host::host().start(address))
            .await
            .map_err(|error| error.to_string())??;
        Ok(())
    }
    pub(super) fn check_running(&mut self) -> Result<(), crate::AppError> {
        match crate::agent_host::host().phase() {
            crate::agent_host::Phase::Failed(error) => Err(error.into()),
            _ => Ok(()),
        }
    }
    pub(super) async fn shutdown(&mut self) -> Result<(), crate::AppError> {
        tokio::task::spawn_blocking(|| crate::desktop_shutdown::stop("headless_exit"))
            .await
            .map_err(|error| error.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_without_an_owned_service_is_a_noop() {
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-stop");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let mut agent = Agent;
        agent.check_running().unwrap();
        agent.shutdown().await.unwrap();
        assert!(!crate::agent_host::host().owns_agent());
    }
}

/// `check_running`'s `Phase::Failed` arm, driven in a **child process**.
///
/// The host is a process-global (`agent_host::HOST`) and a failed start leaves it
/// `Failed` with no way back: nothing in the crate resets it, because production
/// never needs to. Leaving it Failed here would make every later test that starts
/// or inspects the host see a poisoned global, so the case re-execs this same
/// libtest binary with its own `HOME`/`FUTURE_HOME`/socket — the shape
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

    /// A host left `Failed` by a refused start is reported with **that** error,
    /// not silently treated as running — the difference between a server that
    /// says why it cannot serve and one that serves a broken bridge.
    #[test]
    fn a_failed_host_is_reported_with_its_own_error() {
        run_case("failed");
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
        assert_eq!(case, "failed", "unknown case {case}");
        // A held instance lock makes the owned Agent refuse to start, which is how
        // the host reaches `Failed` through its public behaviour rather than
        // through a test-only setter.
        let root = future_rpc::home::future_home();
        std::fs::create_dir_all(root.join("agent")).unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("agent/agent-instance.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();

        // Both are read before the assertion that consumes them: a `match` arm or
        // a second `assert!` that can only fire on a bug would become a residual
        // line of this file, and this file is measured.
        let refused = crate::agent_host::host()
            .start("auto".into())
            .err()
            .map(|error| error.to_string());
        let phase_failed = matches!(
            crate::agent_host::host().phase(),
            crate::agent_host::Phase::Failed(_)
        );
        let mut agent = Agent;
        let reported = agent.check_running().unwrap_err().to_string();
        assert!(
            refused.is_some(),
            "a held instance lock must refuse the start"
        );
        assert!(phase_failed, "the refused start must leave the host Failed");
        assert_eq!(
            reported,
            refused.unwrap(),
            "check_running reports the host's own error"
        );
        eprintln!("{CASE_RAN} {case}");
    }
}
