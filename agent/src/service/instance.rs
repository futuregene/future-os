//! The existing per-data-root singleton and installer-compatible metadata.

use anyhow::{Context, Result};
use future_rpc::instance_lock::{host_name, lock_conflict_message};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

pub(super) struct InstanceFile {
    lock: fd_lock::RwLock<File>,
    path: PathBuf,
}

impl InstanceFile {
    pub(super) fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        Ok(Self {
            lock: fd_lock::RwLock::new(file),
            path,
        })
    }

    pub(super) fn acquire(&mut self, future_home: &Path) -> Result<InstanceGuard<'_>> {
        future_rpc::home::ensure_restore_complete(future_home)?;
        let guard = self.lock.try_write().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                anyhow::anyhow!("{}", lock_conflict_message(&self.path, future_home))
            } else {
                anyhow::Error::from(error)
            }
        })?;
        future_rpc::home::ensure_restore_complete(future_home)?;
        // Preserve the old lock path and fd-lock's Windows first-byte protocol.
        // Never truncate it until the exclusive lock has been acquired.
        guard.set_len(0)?;
        let metadata = match write_metadata(&self.path, future_home) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                eprintln!("Future Agent: could not publish instance metadata: {error:#}");
                None
            }
        };
        Ok(InstanceGuard {
            _metadata: metadata,
            _guard: guard,
        })
    }
}

/// Metadata drops before the lock guard; the borrowed file owner drops last.
/// No allocation is leaked, and callers cannot return the guard from its scope.
pub(super) struct InstanceGuard<'lock> {
    _metadata: Option<InstanceMetadata>,
    _guard: fd_lock::RwLockWriteGuard<'lock, File>,
}

struct InstanceMetadata(PathBuf);

impl Drop for InstanceMetadata {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Future Agent: could not remove instance metadata: {error}");
            }
        }
    }
}

fn write_metadata(lock_path: &Path, future_home: &Path) -> Result<InstanceMetadata> {
    let path = lock_path.with_file_name("agent-instance.json");
    let executable = std::env::current_exe().context("resolve Agent executable")?;
    let mut metadata = serde_json::json!({
        "pid": std::process::id(),
        "executable": executable.to_string_lossy(),
        "futureHome": future_home.to_string_lossy(),
    });
    if let Some(host) = host_name() {
        metadata["host"] = serde_json::json!(host);
    }
    #[cfg(windows)]
    let metadata = {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let mut metadata = metadata;
        let mut created = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        if unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error())
                .context("read Agent process creation time");
        }
        metadata["startTimeFiletime"] = serde_json::json!(
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
        );
        metadata
    };
    std::fs::write(&path, serde_json::to_vec(&metadata)?)
        .with_context(|| format!("write Agent instance metadata {}", path.display()))?;
    Ok(InstanceMetadata(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_instance_lock_is_exclusive_and_reusable_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-instance.lock");
        let mut first_file = InstanceFile::open(path.clone()).unwrap();
        let first = first_file.acquire(dir.path()).unwrap();
        let mut second_file = InstanceFile::open(path).unwrap();
        let error = second_file.acquire(dir.path()).err().unwrap();
        assert!(error.to_string().contains("already running"));
        // The conflicting owner is this process; the message must say where
        // the lock is held and how to get around it.
        let message = error.to_string();
        assert!(
            message.contains("held by"),
            "missing owner identity: {message}"
        );
        assert!(message.contains("kill"), "missing kill guidance: {message}");
        assert!(
            message.contains("FUTURE_HOME"),
            "missing FUTURE_HOME guidance: {message}"
        );
        drop(first);
        let _next = second_file.acquire(dir.path()).unwrap();
    }

    #[test]
    fn instance_metadata_records_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("agent-instance.lock");
        let mut file = InstanceFile::open(lock_path.clone()).unwrap();
        let _guard = file.acquire(dir.path()).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("agent-instance.json")).unwrap())
                .unwrap();
        // gethostname works on every supported platform; the field must be
        // present and non-empty.
        let host = value["host"].as_str().expect("host field");
        assert!(!host.trim().is_empty());
    }

    #[test]
    fn agent_instance_metadata_is_separate_and_removed_before_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("agent-instance.lock");
        let mut file = InstanceFile::open(lock_path.clone()).unwrap();
        let guard = file.acquire(dir.path()).unwrap();
        let path = dir.path().join("agent-instance.json");
        assert_eq!(std::fs::metadata(&lock_path).unwrap().len(), 0);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["pid"], std::process::id());
        assert_eq!(
            value["executable"],
            std::env::current_exe().unwrap().to_string_lossy().as_ref()
        );
        assert_eq!(value["futureHome"], dir.path().to_string_lossy().as_ref());
        #[cfg(windows)]
        assert!(value["startTimeFiletime"].as_u64().is_some());
        // Explicitly drop advisory metadata while the authoritative lock lives.
        drop(guard._metadata);
        assert!(!path.exists());
        let mut second = InstanceFile::open(lock_path).unwrap();
        assert!(second.acquire(dir.path()).is_err());
        drop(guard._guard);
        assert!(second.acquire(dir.path()).is_ok());
    }

    #[test]
    fn rejected_second_owner_preserves_lock_bytes_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-instance.lock");
        let mut first_file = InstanceFile::open(path.clone()).unwrap();
        let first = first_file.acquire(dir.path()).unwrap();
        let metadata_path = path.with_file_name("agent-instance.json");
        let metadata = std::fs::read(&metadata_path).unwrap();
        first._guard.set_len(9).unwrap();
        let mut second = InstanceFile::open(path.clone()).unwrap();
        assert!(second.acquire(dir.path()).is_err());
        assert_eq!(std::fs::metadata(path).unwrap().len(), 9);
        assert_eq!(std::fs::read(metadata_path).unwrap(), metadata);
    }

    #[test]
    fn unremovable_instance_metadata_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-instance.json");
        std::fs::create_dir_all(path.join("nested")).unwrap();
        drop(InstanceMetadata(path.clone()));
        assert!(path.join("nested").is_dir());
        drop(InstanceMetadata(dir.path().join("never-written.json")));
    }

    #[test]
    fn the_instance_lock_creates_its_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("not")
            .join("created")
            .join("agent-instance.lock");
        let mut first_file = InstanceFile::open(path.clone()).unwrap();
        let first = first_file.acquire(dir.path()).unwrap();
        assert!(path.is_file());
        let mut second = InstanceFile::open(path).unwrap();
        assert!(second.acquire(dir.path()).is_err());
        drop(first);
        assert!(second.acquire(dir.path()).is_ok());
        assert!(InstanceFile::open(dir.path().to_owned()).is_err());
    }
}

#[cfg(test)]
mod open_tests {
    use super::*;

    /// The instance lock file may live in a directory that does not exist yet
    /// (`FUTURE_HOME` pointed at a fresh path). Opening must create it, because
    /// the alternative is failing the Agent's startup on a path the user chose.
    #[test]
    fn opening_the_lock_file_creates_its_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("fresh/futureos/agent/agent-instance.lock");
        assert!(!nested.parent().unwrap().exists());
        let opened = InstanceFile::open(nested.clone()).expect("the parent is created");
        assert!(nested.exists(), "the lock file itself is created");
        drop(opened);
        // And the created file is usable as a lock: acquiring twice fails.
        let mut first = InstanceFile::open(nested.clone()).unwrap();
        let guard = first.acquire(dir.path()).unwrap();
        let mut second = InstanceFile::open(nested).unwrap();
        assert!(second.acquire(dir.path()).is_err());
        drop(guard);
    }
}
