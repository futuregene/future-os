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
    pub(super) fn prepare(
        self,
        operation: &super::journal::Operation,
    ) -> Result<PreparedObserver, String> {
        use std::os::windows::process::CommandExt;
        let ready = self.directory.path().join("ready");
        use ::windows::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};
        let mut in_job = Default::default();
        // Determine lifetime constraints before launching, rather than guessing
        // from an undifferentiated CreateProcess error after services stopped.
        unsafe { IsProcessInJob(GetCurrentProcess(), None, &mut in_job) }
            .map_err(|e| format!("Cannot determine update process lifetime: {e}"))?;
        let flags = 0x08000000 | if in_job.as_bool() { 0x01000000 } else { 0 };
        let input_path = self.directory.path().join("stdin");
        std::fs::write(&input_path, []).map_err(|e| format!("Cannot stage observer input: {e}"))?;
        let input = std::fs::File::open(input_path)
            .map_err(|e| format!("Cannot open observer input: {e}"))?;
        let output = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.log)
            .map_err(|e| format!("Cannot open observer diagnostics: {e}"))?;
        let error_output = output
            .try_clone()
            .map_err(|e| format!("Cannot clone observer diagnostics: {e}"))?;
        let mut observer = Command::new(&self.powershell)
            .creation_flags(flags)
            // Real files avoid inheriting invalid handles from a detached GUI.
            .stdin(input)
            .stdout(output)
            .stderr(error_output)
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
            .arg(&self.log)
            .spawn()
            .map_err(|e| {
                format!(
                    "Cannot start update observer {} (creation flags {flags:#x}): {e}",
                    self.powershell.display()
                )
            })?;
        let deadline = Instant::now() + Duration::from_secs(300);
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
        Ok(PreparedObserver {
            installer: Some(self),
            observer,
            record: operation.path(),
            committed: false,
        })
    }
}

// Preparation owns a live observer but grants no installation authority.
// Dropping it on user cancellation or cleanup failure cannot start NSIS.
pub(super) struct PreparedObserver {
    installer: Option<PreparedInstaller>,
    observer: std::process::Child,
    record: PathBuf,
    committed: bool,
}
impl PreparedObserver {
    pub(super) fn commit(mut self, operation: &super::journal::Operation) -> Result<(), String> {
        if self
            .observer
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("Prepared update observer exited; installation was not started".into());
        }
        let installer = self.installer.take().ok_or("Update already committed")?;
        let marker = installer.directory.path().join("commit.tmp");
        std::fs::write(&marker, "commit").map_err(|e| e.to_string())?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&marker)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(marker, installer.directory.path().join("commit"))
            .map_err(|e| e.to_string())?;
        operation.hand_off();
        self.committed = true;
        let _ = installer.directory.keep();
        Ok(())
    }
}
impl Drop for PreparedObserver {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.observer.kill();
            let _ = self.observer.wait();
            let _ = std::fs::remove_dir_all(self.record.with_file_name("program-backup"));
        }
    }
}
