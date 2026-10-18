//! The existing per-data-root singleton and installer-compatible metadata.

use anyhow::{Context, Result};
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

/// The host name of the machine running this Agent, for instance metadata and
/// lock-conflict diagnostics. Empty/unavailable resolves to `None`; the
/// diagnostics degrade gracefully without it.
fn host_name() -> Option<String> {
    host_name_from_parts(hostname_raw(), std::env::var("COMPUTERNAME").ok())
}

#[cfg(unix)]
fn hostname_raw() -> Option<String> {
    let mut buffer = [0u8; 256];
    let length = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if length != 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).trim().to_string())
}

#[cfg(windows)]
fn hostname_raw() -> Option<String> {
    use windows_sys::Win32::Foundation::BOOL;
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    let ok: BOOL = unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) };
    if ok == 0 {
        return None;
    }
    Some(
        String::from_utf16_lossy(&buffer[..size as usize])
            .trim()
            .to_string(),
    )
}

/// Combine the OS host name and (on Windows) `COMPUTERNAME` into one
/// diagnostic value, dropping whitespace-only results. Split from
/// [`host_name`] so the composition is testable without touching the
/// environment or the OS.
fn host_name_from_parts(os_name: Option<String>, env_name: Option<String>) -> Option<String> {
    let os_name = os_name
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let env_name = env_name
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    os_name.or(env_name)
}

/// Build the error shown when the instance lock is already held. The
/// metadata file published by the running owner (best-effort) identifies
/// *where* the Agent runs — crucial on shared-home setups (HPC login and
/// compute nodes share one FutureOS home over NFS), where the conflicting
/// process is on another machine and `kill` cannot reach it: the only local
/// fixes are waiting for that node's job to end or using a different home.
fn lock_conflict_message(lock_path: &Path, future_home: &Path) -> String {
    let owner = std::fs::read(lock_path.with_file_name("agent-instance.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    lock_conflict_message_from(
        lock_path,
        future_home,
        owner.as_ref(),
        host_name().as_deref(),
    )
}

/// [`lock_conflict_message`] with every input injected so both the same-host
/// and cross-host arms are testable without spawning processes or depending
/// on where the test runs.
fn lock_conflict_message_from(
    lock_path: &Path,
    future_home: &Path,
    owner: Option<&serde_json::Value>,
    this_host: Option<&str>,
) -> String {
    let mut message = format!(
        "Future Agent is already running for this FutureOS home (lock: {})",
        lock_path.display()
    );
    let Some(owner) = owner else {
        return message;
    };
    let pid = owner.get("pid").and_then(serde_json::Value::as_u64);
    let host = owner
        .get("host")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let executable = owner
        .get("executable")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let where_ = match (&host, pid) {
        (Some(host), Some(pid)) => format!("pid {pid} on host {host}"),
        (Some(host), None) => format!("host {host}"),
        (None, Some(pid)) => format!("pid {pid}"),
        (None, None) => String::new(),
    };
    if !where_.is_empty() {
        message.push_str(&format!("\n  held by: {where_}"));
    }
    if let Some(executable) = executable {
        message.push_str(&format!("\n  executable: {executable}"));
    }
    // A lock held on another host cannot be killed locally; only a
    // different FutureOS home (or waiting for that process to exit) helps.
    let cross_host = host
        .as_deref()
        .zip(this_host)
        .is_some_and(|(owner_host, this_host)| owner_host != this_host);
    if cross_host {
        let owner_host = host.as_deref().unwrap_or_default();
        message.push_str(&format!(
            "\n  the lock is held on a different host ({owner_host}); this machine \
             cannot stop that process — wait for it to exit, or run this Agent \
             with its own FutureOS home, e.g.:"
        ));
    } else {
        message.push_str(&format!(
            "\n  to stop it: kill {} (or close the owning app), or run this Agent \
             with its own FutureOS home, e.g.:",
            pid.map(|p| p.to_string())
                .unwrap_or_else(|| "<pid>".to_string())
        ));
    }
    message.push_str(&format!(
        "\n    future agent --home {}.future-{}",
        future_home.display(),
        host_token_example()
    ));
    message.push_str(&format!(
        "\n  metadata (advisory, may be stale after a crash): {}",
        lock_path.with_file_name("agent-instance.json").display()
    ));
    message
}

/// Placeholder used in the suggested `--home` example command. The example
/// shows how to derive a per-host sibling directory; `$(hostname)` is shell
/// syntax, `%COMPUTERNAME%` is the cmd equivalent.
fn host_token_example() -> &'static str {
    if cfg!(windows) {
        "%COMPUTERNAME%"
    } else {
        "$(hostname)"
    }
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
            message.contains("--home"),
            "missing --home guidance: {message}"
        );
        drop(first);
        let _next = second_file.acquire(dir.path()).unwrap();
    }

    #[test]
    fn host_name_prefers_the_os_and_ignores_blanks() {
        assert_eq!(
            host_name_from_parts(Some("intel080".into()), None),
            Some("intel080".to_string())
        );
        assert_eq!(
            host_name_from_parts(Some("  ".into()), Some("WIN-PC".into())),
            Some("WIN-PC".to_string())
        );
        assert_eq!(host_name_from_parts(None, Some("".into())), None);
        assert_eq!(host_name_from_parts(None, None), None);
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
    fn cross_host_conflict_explains_the_lock_is_out_of_reach() {
        let lock = std::path::Path::new("/nfs/home/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/nfs/home/.future");
        let owner = serde_json::json!({
            "pid": 4242,
            "host": "intel080",
            "executable": "/nfs/home/bin/future-agent",
        });
        let message = lock_conflict_message_from(lock, home, Some(&owner), Some("login01"));
        assert!(message.contains("pid 4242 on host intel080"));
        assert!(message.contains("different host"));
        assert!(message.contains("--home"));
        assert!(
            !message.contains("kill 4242"),
            "must not suggest killing a remote process"
        );
    }

    #[test]
    fn same_host_conflict_suggests_killing_the_owner() {
        let lock = std::path::Path::new("/home/u/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/home/u/.future");
        let owner = serde_json::json!({"pid": 4242, "host": "intel080"});
        let message = lock_conflict_message_from(lock, home, Some(&owner), Some("intel080"));
        assert!(message.contains("held by: pid 4242 on host intel080"));
        assert!(message.contains("kill 4242"));
        assert!(message.contains("--home"));
    }

    #[test]
    fn owner_description_covers_host_only_pid_only_and_neither() {
        let lock = std::path::Path::new("/h/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/h/.future");

        // Host without a pid, on another host: the owner is named by host alone
        // and the cross-host wording follows (no pid is needed to explain it).
        let only_host = lock_conflict_message_from(
            lock,
            home,
            Some(&serde_json::json!({"host": "intel080"})),
            Some("login01"),
        );
        assert!(only_host.contains("held by: host intel080"), "{only_host}");
        assert!(only_host.contains("different host"), "{only_host}");

        // Pid without a host, same host as ours: the owner is named by pid, and
        // the local advice uses the pid we have.
        let only_pid = lock_conflict_message_from(
            lock,
            home,
            Some(&serde_json::json!({"pid": 7})),
            host_name().as_deref(),
        );
        assert!(only_pid.contains("held by: pid 7"), "{only_pid}");
        assert!(only_pid.contains("kill 7"), "{only_pid}");

        // Neither: there is no owner to name, so the message must not invent a
        // "held by" line — the location advice stays, with a placeholder pid.
        let neither = lock_conflict_message_from(
            lock,
            home,
            Some(&serde_json::json!({"pid": null, "host": null})),
            Some("login01"),
        );
        assert!(!neither.contains("held by:"), "{neither}");
        assert!(neither.contains("kill <pid>"), "{neither}");
    }

    #[test]
    fn missing_or_unreadable_metadata_degrades_to_the_plain_message() {
        let lock = std::path::Path::new("/h/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/h/.future");
        let message = lock_conflict_message_from(lock, home, None, Some("intel080"));
        assert_eq!(
            message,
            "Future Agent is already running for this FutureOS home (lock: /h/.future/agent/agent-instance.lock)"
        );
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
    /// (`--home` pointed at a fresh path). Opening must create it, because the
    /// alternative is failing the Agent's startup on a path the user just chose.
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
