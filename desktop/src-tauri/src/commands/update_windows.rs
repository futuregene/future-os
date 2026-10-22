//! Signed raw NSIS payload handoff. The stock updater exits even when
//! ShellExecute fails; our observer checks launch and installer exit status.
use crate::AppError;
use std::{
    io,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

const OBSERVER_FLAGS: u32 = 0x08000000 | 0x01000000;

fn spawn_observer_with<T>(
    mut spawn: impl FnMut(u32) -> io::Result<T>,
    is_in_job: impl FnOnce() -> io::Result<bool>,
    report: impl FnOnce(&str),
) -> io::Result<T> {
    match spawn(OBSERVER_FLAGS) {
        // ERROR_NOT_SUPPORTED can reject the breakaway launch. Only retry
        // outside a Job: dropping breakaway inside one can kill the observer
        // with Desktop, leaving the installation without recovery supervision.
        Err(first) if first.raw_os_error() == Some(50) => {
            let in_job = is_in_job().map_err(|query| {
                io::Error::new(
                    query.kind(),
                    format!("{first}; cannot verify safe observer fallback: {query}"),
                )
            })?;
            if in_job {
                return Err(first);
            }
            report(&format!(
                "Update observer breakaway launch failed: {first}; retrying outside a Job."
            ));
            spawn(0x08000000).map_err(|retry| {
                io::Error::new(
                    retry.kind(),
                    format!("{retry} (initial breakaway launch: {first})"),
                )
            })
        }
        result => result,
    }
}

fn current_process_is_in_job() -> io::Result<bool> {
    use ::windows::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};
    let mut in_job = Default::default();
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle; in_job is a
    // writable BOOL, and a null job asks about membership in any Job.
    unsafe { IsProcessInJob(GetCurrentProcess(), None, &mut in_job) }.map_err(io::Error::other)?;
    Ok(in_job.as_bool())
}

pub(super) struct PreparedInstaller {
    directory: tempfile::TempDir,
    desktop: PathBuf,
    powershell: PathBuf,
    log: PathBuf,
}
impl PreparedInstaller {
    pub(super) fn new(bytes: &[u8]) -> Result<Self, AppError> {
        // Current release packaging publishes raw NSIS EXEs. Reject other
        // signed formats before stopping work instead of guessing a launcher.
        let pe_offset = bytes
            .get(0x3c..0x40)
            .map(|offset| u32::from_le_bytes(offset.try_into().unwrap()) as usize);
        if !bytes.starts_with(b"MZ")
            || !pe_offset.is_some_and(|offset| {
                offset.checked_add(4).and_then(|end| bytes.get(offset..end)) == Some(&b"PE\0\0"[..])
            })
        {
            return Err("The Windows update must be a signed raw NSIS executable.".into());
        }
        let directory = tempfile::Builder::new()
            .prefix("futureos-update-")
            .tempdir()
            .map_err(|e| super::updater_error("Cannot stage update", e))?;
        let desktop = std::env::current_exe()
            .map_err(|e| super::updater_error("Cannot locate Desktop", e))?;
        let powershell = PathBuf::from(
            std::env::var_os("SystemRoot")
                .ok_or_else(|| AppError::Message("SystemRoot is missing.".into()))?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let log = future_rpc::home::future_home().join("app/logs/update-recovery.log");
        std::fs::create_dir_all(log.parent().unwrap())
            .map_err(|e| super::updater_error("Cannot create update log", e))?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| super::updater_error("Cannot write update diagnostics", e))?;
        std::fs::write(directory.path().join("installer.exe"), bytes)
            .and_then(|()| {
                std::fs::write(
                    directory.path().join("launch.ps1"),
                    include_str!("../../windows/update-launcher.ps1"),
                )
            })
            .map_err(|e| super::updater_error("Cannot stage update", e))?;
        let arguments = std::env::args_os()
            .skip(1)
            .map(|arg| super::quote_windows_argument(&arg.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        std::fs::write(directory.path().join("resume-args.txt"), arguments)
            .map_err(|e| super::updater_error("Cannot stage restart arguments", e))?;
        Ok(Self {
            directory,
            desktop,
            powershell,
            log,
        })
    }
    pub(super) fn launch(self, operation: &super::journal::Operation) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        let ready = self.directory.path().join("ready");
        let mut command = Command::new(&self.powershell);
        command
            // The GUI has detached its console; the observer writes its own log.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(self.directory.path().join("launch.ps1"))
            .arg("-ParentPid")
            .arg(std::process::id().to_string())
            .arg("-Desktop")
            .arg(&self.desktop)
            .arg("-Directory")
            .arg(self.directory.path())
            .arg("-Record")
            .arg(operation.path())
            .arg("-Log")
            .arg(&self.log);
        let mut observer = spawn_observer_with(
            |flags| command.creation_flags(flags).spawn(),
            current_process_is_in_job,
            crate::host_environment::report,
        )
        .map_err(|e| format!("Cannot start update observer: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            if observer
                .try_wait()
                .map_err(|e| {
                    let _ = observer.kill();
                    let _ = observer.wait();
                    e.to_string()
                })?
                .is_some()
            {
                return Err(format!(
                    "Update observer failed; see {}",
                    self.log.display()
                ));
            }
            if Instant::now() >= deadline {
                let _ = observer.kill();
                let _ = observer.wait();
                return Err("Update observer readiness timed out; no installation started.".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if observer
            .try_wait()
            .map_err(|e| {
                let _ = observer.kill();
                let _ = observer.wait();
                e.to_string()
            })?
            .is_some()
        {
            return Err(format!(
                "Update observer exited before handoff; see {}",
                self.log.display()
            ));
        }
        // Observer owns staged files until the installer exits. Dropping the
        // Rust Child does not kill it; it must survive Desktop's normal exit.
        operation.hand_off();
        let _ = self.directory.keep();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_breakaway_retries_only_outside_a_job() {
        for in_job in [false, true] {
            let mut attempts = Vec::new();
            let result = spawn_observer_with(
                |flags| {
                    attempts.push(flags);
                    if flags == OBSERVER_FLAGS {
                        Err(io::Error::from_raw_os_error(50))
                    } else {
                        Ok(())
                    }
                },
                || Ok(in_job),
                |_| {},
            );
            assert_eq!(result.is_ok(), !in_job);
            assert_eq!(attempts.len(), if in_job { 1 } else { 2 });
            if !in_job {
                assert_eq!(attempts[1], 0x08000000);
            }
        }
    }

    #[test]
    fn other_launch_errors_never_retry() {
        for error in [2, 5, 6, 193] {
            let result = spawn_observer_with::<()>(
                |_| Err(io::Error::from_raw_os_error(error)),
                || panic!("only unsupported launches should query the Job"),
                |_| panic!("unexpected fallback"),
            );
            assert_eq!(result.unwrap_err().raw_os_error(), Some(error));
        }
    }

    #[test]
    fn successful_launch_does_not_query_job_membership() {
        assert_eq!(
            spawn_observer_with(
                |_| Ok(42),
                || panic!("successful launches need no fallback"),
                |_| panic!("unexpected fallback"),
            )
            .unwrap(),
            42
        );
    }

    #[test]
    fn retry_failure_keeps_both_errors() {
        let mut attempts = 0;
        let error = spawn_observer_with::<()>(
            |_| {
                attempts += 1;
                Err(io::Error::from_raw_os_error(if attempts == 1 {
                    50
                } else {
                    2
                }))
            },
            || Ok(false),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(attempts, 2);
        assert!(error.to_string().contains("initial breakaway launch:"));
        assert!(error.to_string().contains("50"));
        assert!(error.to_string().contains("2"));
    }

    #[test]
    fn unknown_job_membership_never_retries() {
        let mut attempts = 0;
        let result = spawn_observer_with::<()>(
            |_| {
                attempts += 1;
                Err(io::Error::from_raw_os_error(50))
            },
            || Err(io::Error::from_raw_os_error(5)),
            |_| panic!("unknown Job membership must fail closed"),
        );
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}
