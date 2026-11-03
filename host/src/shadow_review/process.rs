//! Interruptible Git execution shared by capture, review and retention.
//! Files rather than pipes let large patches drain without a blocked reader or
//! an untracked helper thread. Only this exact child is stopped, never a tree.

use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::Error;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_LIMIT: u64 = 64 * 1024 * 1024;

struct ReapedChild(Child);
impl Deref for ReapedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl DerefMut for ReapedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for ReapedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Shared cancellation boundary for capture and projection serialization.
pub(super) fn with_lock<T>(
    lock: &std::sync::Mutex<()>,
    job: impl FnOnce() -> Result<T, Error>,
) -> Result<T, Error> {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        if crate::request_scope::is_stopping() {
            return Err("Review cancelled because the host is stopping.".into());
        }
        match lock.try_lock() {
            Ok(_guard) => return job(),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let _guard = poisoned.into_inner();
                return job();
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err("Timed out waiting for review work to finish.".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

pub(super) fn run(command: &mut Command, input: Option<&[u8]>) -> Result<Output, Error> {
    run_with_policy(
        command,
        input,
        COMMAND_TIMEOUT,
        crate::request_scope::is_stopping,
    )
}

fn run_with_policy(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<Output, Error> {
    if cancelled() {
        return Err("Shadow Git cancelled because the host is stopping.".into());
    }
    let mut stdin = tempfile::tempfile()?;
    if let Some(bytes) = input {
        stdin.write_all(bytes)?;
        stdin.rewind()?;
    }
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command.stdin(Stdio::from(stdin));
    command.stdout(stdout.try_clone()?);
    command.stderr(stderr.try_clone()?);
    let mut child = ReapedChild(
        command
            .spawn()
            .map_err(|e| format!("Shadow Git failed to spawn: {e}"))?,
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        // Reap a child even on cancellation/errors: dropping std::process::Child
        // alone leaves it running and keeps its workspace/file locks alive.
        let failure = if cancelled() {
            Some("Shadow Git cancelled because the host is stopping.".to_string())
        } else if Instant::now() >= deadline {
            Some(format!(
                "Shadow Git exceeded its execution deadline ({timeout:?})."
            ))
        } else if stdout.metadata()?.len() > OUTPUT_LIMIT || stderr.metadata()?.len() > OUTPUT_LIMIT
        {
            Some("Shadow Git output exceeded the review limit.".to_string())
        } else {
            None
        };
        if let Some(error) = failure {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        }
    };
    fn read(file: &mut std::fs::File) -> Result<Vec<u8>, Error> {
        if file.metadata()?.len() > OUTPUT_LIMIT {
            return Err("Shadow Git output exceeded the review limit.".into());
        }
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err("Shadow Git output exceeded the review limit.".into());
        }
        Ok(bytes)
    }
    Ok(Output {
        status,
        stdout: read(&mut stdout)?,
        stderr: read(&mut stderr)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::NoWindow;

    const FIXTURE: &str = "FUTURE_SHADOW_PROCESS_FIXTURE";
    fn child(mode: &str, marker: &std::path::Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .no_window()
            .args([
                "--exact",
                "shadow_review::process::tests::child_fixture",
                "--nocapture",
            ])
            .env(FIXTURE, mode)
            .env("FUTURE_SHADOW_PROCESS_MARKER", marker);
        command
    }
    #[test]
    fn child_fixture() {
        let Some(mode) = std::env::var_os(FIXTURE) else {
            return;
        };
        if mode == "echo" {
            let mut bytes = Vec::new();
            std::io::stdin().read_to_end(&mut bytes).unwrap();
            std::io::stdout().write_all(&bytes).unwrap();
            std::io::stderr().write_all(b"diagnostic").unwrap();
        } else {
            std::fs::write(
                std::env::var_os("FUTURE_SHADOW_PROCESS_MARKER").unwrap(),
                "ready",
            )
            .unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
    }
    #[test]
    fn large_input_and_output_cannot_deadlock_on_full_pipes() {
        let directory = tempfile::tempdir().unwrap();
        let data = vec![b'x'; 1024 * 1024];
        let output = run_with_policy(
            &mut child("echo", directory.path()),
            Some(&data),
            Duration::from_secs(30),
            || false,
        )
        .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.windows(data.len()).any(|w| w == data));
        assert!(String::from_utf8_lossy(&output.stderr).contains("diagnostic"));
    }
    #[test]
    fn cancellation_after_child_starts_stops_and_reaps_it() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("ready");
        let error = run_with_policy(
            &mut child("wait", &marker),
            None,
            Duration::from_secs(30),
            || marker.exists(),
        )
        .unwrap_err();
        assert!(
            marker.exists(),
            "the child actually started before cancellation"
        );
        assert!(error.to_string().contains("cancelled"), "{error}");
    }
    #[test]
    fn a_command_has_a_deadline_even_without_a_stop_request() {
        let directory = tempfile::tempdir().unwrap();
        let error = run_with_policy(
            &mut child("wait", &directory.path().join("ready")),
            None,
            Duration::from_millis(50),
            || false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("deadline"), "{error}");
    }
}
