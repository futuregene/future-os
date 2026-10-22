//! Same-volume, atomic bundle exchange. The previous bundle remains available
//! until a matching new process confirms GUI and Agent readiness.
use super::{
    journal,
    process::{self, Completion, PreparedProcess},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn bundle() -> Result<PathBuf, String> {
    tauri_plugin_updater::extract_path_from_executable(
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
fn staging(target: &Path, id: &str) -> Result<PathBuf, String> {
    if !id.starts_with("update-") || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("Invalid transaction identifier".into());
    }
    Ok(target
        .parent()
        .ok_or("App bundle has no parent")?
        .join(format!(".futureos-{id}")))
}
pub(super) fn can_prepare() -> bool {
    use std::os::unix::ffi::OsStrExt;
    bundle()
        .ok()
        .and_then(|path| {
            path.parent()
                .and_then(|p| std::ffi::CString::new(p.as_os_str().as_bytes()).ok())
        })
        .is_some_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 })
}
fn sync_tree(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            sync_tree(&entry.map_err(|e| e.to_string())?.path())?;
        }
    }
    std::fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())
}
fn probe(path: &Path) -> Result<(), String> {
    let mut child = Command::new(path)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Candidate executable cannot start: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("Candidate executable probe failed: {status}"))
            };
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Candidate executable probe timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
pub(super) struct Prepared {
    helper: Option<PreparedProcess>,
    directory: PathBuf,
    committed: bool,
}
impl Prepared {
    pub(super) fn new(bytes: &[u8], operation: &journal::Operation) -> Result<Self, String> {
        let target = bundle()?;
        if target.extension().and_then(|s| s.to_str()) != Some("app") {
            return Err(
                "Install the application in a writable Applications folder before updating".into(),
            );
        }
        let id = operation
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .ok_or("Missing transaction directory")?
            .to_string_lossy()
            .into_owned();
        let directory = staging(&target, &id)?;
        std::fs::create_dir(&directory)
            .map_err(|e| format!("Cannot stage update beside the application: {e}"))?;
        let prepared = (|| {
            let decoder = flate2::read::GzDecoder::new(bytes);
            tar::Archive::new(decoder)
                .unpack(&directory)
                .map_err(|e| e.to_string())?;
            let entries = std::fs::read_dir(&directory)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if entries.len() != 1
                || entries[0].path().extension().and_then(|s| s.to_str()) != Some("app")
            {
                return Err("Update must contain exactly one application bundle".into());
            }
            let candidate = directory.join("candidate.app");
            std::fs::rename(entries[0].path(), &candidate).map_err(|e| e.to_string())?;
            let verification = Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&candidate)
                .output()
                .map_err(|e| e.to_string())?;
            if !verification.status.success() {
                return Err(format!(
                    "Candidate application signature is invalid: {}",
                    String::from_utf8_lossy(&verification.stderr)
                ));
            }
            let executable = Command::new("/usr/libexec/PlistBuddy")
                .args(["-c", "Print :CFBundleExecutable"])
                .arg(candidate.join("Contents/Info.plist"))
                .output()
                .map_err(|e| e.to_string())?;
            if !executable.status.success() {
                return Err("Candidate bundle has no executable identity".into());
            }
            let executable = String::from_utf8(executable.stdout).map_err(|e| e.to_string())?;
            let name = executable.trim();
            if name.is_empty() || name.contains('/') || name.contains('\\') {
                return Err("Invalid candidate executable identity".into());
            }
            probe(&candidate.join("Contents/MacOS").join(name))?;
            probe(&candidate.join("Contents/MacOS/future"))?;
            sync_tree(&directory)?;
            let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
            command
                .arg("--app-update-macos-worker")
                .arg(&target)
                .arg(&id)
                .arg(operation.path());
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(operation.path().with_file_name("worker.log"))
                .map_err(|e| e.to_string())?;
            command.stderr(log);
            PreparedProcess::start(command)
        })();
        match prepared {
            Ok(helper) => Ok(Self {
                helper: Some(helper),
                directory,
                committed: false,
            }),
            Err(error) => {
                let _ = std::fs::remove_dir_all(directory);
                Err(error)
            }
        }
    }
    pub(super) fn commit(mut self) -> Result<Completion, String> {
        self.committed = true;
        // Keep the directory after commit, including when the parent crashes.
        let helper = self
            .helper
            .take()
            .ok_or("Update helper was already committed")?;
        let result = helper.commit();
        if result.as_ref().is_ok_and(|completion| completion.restored) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
        result
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}
fn exchange(a: &Path, b: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let a = std::ffi::CString::new(a.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let b = std::ffi::CString::new(b.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // Both bundles were prepared in the same parent filesystem. RENAME_SWAP
    // keeps the installed pathname valid across crashes and needs no delete.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}
pub(super) fn worker(args: &[String]) -> Result<(), String> {
    let [target, id, record] = args else {
        return Err("Invalid macOS update worker arguments".into());
    };
    let target = PathBuf::from(target);
    let candidate = staging(&target, id)?.join("candidate.app");
    let record = PathBuf::from(record);
    let _lease = journal::observer_lease(&record).map_err(|e| e.to_string())?;
    if !process::await_commit()? {
        if let Some(directory) = candidate.parent() {
            let _ = std::fs::remove_dir_all(directory);
        }
        return Ok(());
    }
    let _agent_lease = match process::agent_lease(&record) {
        Ok(lease) => lease,
        Err(error) => {
            let completion = Completion {
                ok: false,
                restored: true,
                diagnostic: error,
            };
            journal::write_worker_result(
                &record,
                "failed-restored",
                Some(completion.diagnostic.clone()),
                true,
            )
            .map_err(|e| e.to_string())?;
            return process::complete(&completion);
        }
    };
    let completion = match exchange(&target, &candidate) {
        Ok(()) => {
            let durable = (|| {
                std::fs::File::open(target.parent().ok_or("Missing application parent")?)
                    .and_then(|file| file.sync_all())
                    .map_err(|e| e.to_string())?;
                std::fs::File::open(candidate.parent().ok_or("Missing candidate parent")?)
                    .and_then(|file| file.sync_all())
                    .map_err(|e| e.to_string())?;
                journal::write_worker_result(&record, "pending-validation", None, true)
                    .map_err(|e| e.to_string())
            })();
            if let Err(error) = durable {
                let restored = exchange(&target, &candidate).is_ok();
                Completion {
                    ok: false,
                    restored,
                    diagnostic: format!("Cannot persist installation result: {error}"),
                }
            } else {
                Completion::success()
            }
        }
        Err(error) => Completion {
            ok: false,
            restored: true,
            diagnostic: format!(
                "Atomic application exchange failed; original bundle is intact: {error}"
            ),
        },
    };
    process::complete(&completion)
}
pub(super) fn confirm(id: &str) -> Result<(), String> {
    let directory = staging(&bundle()?, id)?;
    if directory.exists() {
        std::fs::remove_dir_all(directory).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_exchange_retains_both_complete_versions_and_can_be_reversed() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed.app");
        let candidate = root.path().join("candidate.app");
        std::fs::create_dir(&installed).unwrap();
        std::fs::create_dir(&candidate).unwrap();
        std::fs::write(installed.join("version"), b"old").unwrap();
        std::fs::write(candidate.join("version"), b"new").unwrap();
        exchange(&installed, &candidate).unwrap();
        assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"new");
        assert_eq!(std::fs::read(candidate.join("version")).unwrap(), b"old");
        exchange(&installed, &candidate).unwrap();
        assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"old");
    }
    #[test]
    fn failed_exchange_leaves_the_installed_version_in_place() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed.app");
        std::fs::create_dir(&installed).unwrap();
        std::fs::write(installed.join("version"), b"old").unwrap();
        assert!(exchange(&installed, &root.path().join("missing.app")).is_err());
        assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"old");
    }
}
