//! Signed raw NSIS payload handoff. The stock updater exits even when
//! ShellExecute fails; our observer checks launch and installer exit status.
use crate::AppError;
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

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
    pub(super) fn launch(self) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        let ready = self.directory.path().join("ready");
        let mut observer = Command::new(&self.powershell)
            // CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB. A restrictive
            // parent job must fail launch, rather than kill recovery on exit.
            .creation_flags(0x08000000 | 0x01000000)
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
            .arg("-Log")
            .arg(&self.log)
            .spawn()
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
        let _ = self.directory.keep();
        Ok(())
    }
}
