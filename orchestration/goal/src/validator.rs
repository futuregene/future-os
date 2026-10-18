//! Bounded validator subprocesses. Drain both pipes even after the retained
//! diagnostic budget is exhausted. Cancellation kills the whole process tree.
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

const OUTPUT_BYTES: usize = 16_384;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Summary prefix for a validator that never produced a verdict (the shell
/// could not start it, or could not even parse the command line). The ledger
/// otherwise records only the exit status, and a bare `exit 2` cannot tell a
/// rejected artifact from a malformed `--verify` string.
const COULD_NOT_RUN_PREFIX: &str = "validator could not run";

/// Whether a failed receipt says the validator never ran. The status stays
/// `Failed` on purpose: a gate that cannot start still must not complete the
/// todo, and the validation budget must still bound a command that can never
/// pass. Only the reported reason changes.
pub fn could_not_run(receipt: &crate::state::TaskValidation) -> bool {
    receipt.summary.starts_with(COULD_NOT_RUN_PREFIX)
}

/// Exit statuses that say "the shell never executed the command" rather than
/// "the command judged the artifact": POSIX shells use 126 (found but not
/// executable) and 127 (not found); cmd.exe uses 9009. A command line the
/// shell cannot parse exits 2 before running anything — but exit 2 is also a
/// legitimate validator status (`grep` exits 2 on a read error), so require
/// the shell's own diagnostic.
fn shell_could_not_run(code: i32, stderr: &[u8]) -> bool {
    match code {
        126 | 127 | 9009 => true,
        2 => String::from_utf8_lossy(stderr)
            .to_lowercase()
            .contains("syntax error"),
        _ => false,
    }
}

struct ProcessTree(u32);
impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: the child creates its own process group, whose id is its pid.
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &self.0.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

async fn drain(mut pipe: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let n = pipe.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        retained.extend_from_slice(&chunk[..n]);
        if retained.len() > OUTPUT_BYTES {
            retained.drain(..retained.len() - OUTPUT_BYTES);
        }
    }
    Ok(retained)
}

pub async fn validate(
    cwd: &Path,
    command: &str,
    timeout: Duration,
) -> crate::state::TaskValidation {
    use crate::state::{task_validation_receipt, RecoveryKind, ValidationStatus};
    let result = execute(cwd, command, timeout).await;
    let (status, summary, exit) = match result {
        Ok((code, stdout, stderr)) => {
            let output = format!(
                "stdout (tail):\n{}\nstderr (tail):\n{}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
            match code {
                Some(0) => (
                    ValidationStatus::Passed,
                    format!("validator passed (exit 0)\n{output}"),
                    code,
                ),
                Some(exit) if shell_could_not_run(exit, &stderr) => (
                    ValidationStatus::Failed,
                    format!(
                        "{COULD_NOT_RUN_PREFIX}: the shell could not start the command (exit {exit}) \
                         — this is not a verdict on the artifact; fix the --verify command\n{output}"
                    ),
                    code,
                ),
                _ => (
                    ValidationStatus::Failed,
                    format!(
                        "validator exited {} — repair output, not acceptance criteria\n{output}",
                        code.unwrap_or(-1)
                    ),
                    code,
                ),
            }
        }
        Err(error) => (
            ValidationStatus::Inconclusive,
            format!("validator failed to run: {error}"),
            None,
        ),
    };
    let recovery = (status != ValidationStatus::Passed).then_some(RecoveryKind::RepairRequired);
    task_validation_receipt(status, command, &summary, recovery, exit)
}

async fn execute(
    cwd: &Path,
    command: &str,
    timeout: Duration,
) -> anyhow::Result<(Option<i32>, Vec<u8>, Vec<u8>)> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd.exe");
        c.args(["/D", "/S", "/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let tree = ProcessTree(
        child
            .id()
            .ok_or_else(|| anyhow::anyhow!("validator has no pid"))?,
    );
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // One timeout covers execution AND pipe draining (a descendant can inherit
    // a pipe after its parent exits). Dropping these futures closes the pipes.
    let result = tokio::time::timeout(timeout, async {
        let (status, out, err) = tokio::try_join!(child.wait(), drain(stdout), drain(stderr))?;
        Ok::<_, std::io::Error>((status.code(), out, err))
    })
    .await;
    drop(tree);
    match result {
        Ok(result) => Ok(result?),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!(
                "timeout after {}ms; process tree terminated",
                timeout.as_millis()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failure_carries_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = validate(
            dir.path(),
            "echo diagnostic 1>&2 & exit 7",
            Duration::from_secs(5),
        )
        .await;
        assert!(!receipt.ok);
        assert_eq!(receipt.exit_code, Some(7));
        assert!(receipt.summary.contains("diagnostic"));
        // A command that ran and returned a verdict is NOT "could not run".
        assert!(!could_not_run(&receipt), "{}", receipt.summary);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_command_is_reported_as_could_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = validate(
            dir.path(),
            "future-goal-validator-that-does-not-exist --check",
            Duration::from_secs(5),
        )
        .await;
        assert!(!receipt.ok);
        assert_eq!(receipt.exit_code, Some(127));
        assert!(could_not_run(&receipt), "{}", receipt.summary);
        assert!(
            receipt.summary.to_lowercase().contains("not found"),
            "stderr must be retained: {}",
            receipt.summary
        );
    }

    /// The exact shape that rejected five tickets: an unsubstituted
    /// placeholder (`--report <pinned union>`) reads to `sh` as a redirection,
    /// so the command line never parses and nothing runs. Exit 2 alone is not
    /// proof of that — the shell's own diagnostic has to back it.
    #[cfg(unix)]
    #[tokio::test]
    async fn shell_syntax_error_is_could_not_run_not_a_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = validate(
            dir.path(),
            "python3 reconcile.py --report <pinned union> ; python3 staletable.py | grep '^tauri-appdata'",
            Duration::from_secs(5),
        )
        .await;
        assert!(!receipt.ok);
        assert_eq!(receipt.exit_code, Some(2));
        assert!(could_not_run(&receipt), "{}", receipt.summary);
        // dash prints `Syntax error: ...`, bash/sh `syntax error near ...`.
        assert!(
            receipt.summary.to_lowercase().contains("syntax error"),
            "stderr must be retained: {}",
            receipt.summary
        );
    }

    /// Exit 2 without a shell parse error is a verdict (or at least the
    /// command's own usage error), not "the shell could not start it".
    #[cfg(unix)]
    #[tokio::test]
    async fn valid_exit_2_is_not_classified_as_could_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = validate(
            dir.path(),
            "echo no-match 1>&2; exit 2",
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(receipt.exit_code, Some(2));
        assert!(!could_not_run(&receipt), "{}", receipt.summary);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn large_stderr_is_drained_and_retention_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = validate(
            dir.path(),
            "i=0; while [ $i -lt 20000 ]; do echo error-message >&2; i=$((i+1)); done; exit 2",
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(receipt.exit_code, Some(2));
        assert!(receipt.summary.contains("error-message"));
        assert!(receipt.summary.len() < OUTPUT_BYTES + 1000);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_validation_reaps_the_owned_process() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        let task = tokio::spawn(async move {
            validate(
                &cwd,
                "echo $$ > validator.pid; sleep 60",
                Duration::from_secs(60),
            )
            .await
        });
        let pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pid) = std::fs::read_to_string(dir.path().join("validator.pid"))
                    .ok()
                    .and_then(|text| text.trim().parse::<u32>().ok())
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(5), async {
            while crate::compat::pid_alive(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled validator process must be killed and reaped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hung_child_and_inherited_pipe_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        for command in ["sleep 60", "sleep 60 & exit 0"] {
            let receipt = validate(dir.path(), command, Duration::from_millis(50)).await;
            assert!(!receipt.ok);
            assert!(receipt.summary.contains("timeout"), "{}", receipt.summary);
        }
    }
}
