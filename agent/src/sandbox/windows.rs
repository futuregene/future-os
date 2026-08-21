//! Windows sandbox executor (SANDBOX_PLAN §11), `#[cfg(windows)]` only.
//!
//! W2/W3 land in slices so each can be compiled/verified on Windows:
//!   1. **Job Object** (this file so far) — process-tree teardown, the Windows
//!      analog of the unix process-group kill. Needed for every shell run, not
//!      just sandboxed ones, so it ships before the enforcement layer.
//!   2. restricted token + capability SID derivation — W2, kept disconnected
//!      from product execution until the Windows AccessCheck matrix passes.
//!   3. additive FutureOS-owned ACE application/audit — W2, likewise opt-in.
//!   4. suspended launch glue — W3.

#[path = "windows/acl.rs"]
mod acl;
#[path = "windows/audit.rs"]
mod audit;
#[cfg(test)]
#[path = "windows/integration_tests.rs"]
mod integration_tests;
#[path = "windows/process.rs"]
pub(crate) mod process;
#[path = "windows/runner.rs"]
pub(crate) mod runner;
#[path = "windows/token.rs"]
mod token;

use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// A job object configured to kill every member process when the handle closes.
/// Assign a freshly-spawned shell PID; dropping the job (or calling
/// [`Job::terminate`]) then kills the shell and all of its descendants as a tree.
pub struct Job(HANDLE);

// A raw job HANDLE is safe to move/hand across the spawn future.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// Create an anonymous `KILL_ON_JOB_CLOSE` job object.
    pub fn create() -> io::Result<Self> {
        Self::create_with_flags(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK)
    }

    /// Sandbox jobs never allow descendants to break away. This is separate
    /// from the legacy general-purpose Job because detached browsers are still
    /// supported for unsandboxed shell runs.
    pub(crate) fn create_sandbox() -> io::Result<Self> {
        Self::create_with_flags(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
    }

    fn create_with_flags(limit_flags: u32) -> io::Result<Self> {
        // SAFETY: null attributes/name request an anonymous, default-secured job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job(handle);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        // Chrome starts sandboxed renderer, GPU, and network processes that
        // request to leave their parent's job. Allow that explicit breakaway
        // while retaining kill-on-close for every process that stays in the
        // agent's job.
        info.BasicLimitInformation.LimitFlags = limit_flags;
        // SAFETY: `info` is a correctly-sized, initialized struct for this class.
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            // `job` drops here, closing the handle.
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Assign a process (by PID) to this job so it and its future children die
    /// with the job. Best-effort — a failure just means no tree-kill this run.
    pub fn assign(&self, pid: u32) -> io::Result<()> {
        // SAFETY: FFI. Handle is closed before returning.
        let process = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SET_QUOTA, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let result = self.assign_handle(process);
        unsafe { CloseHandle(process) };
        result
    }

    pub(crate) fn assign_handle(&self, process: HANDLE) -> io::Result<()> {
        let ok = unsafe { AssignProcessToJobObject(self.0, process) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Kill every process in the job immediately (abort / timeout teardown).
    pub fn terminate(&self) {
        // SAFETY: FFI on an owned job handle.
        unsafe { TerminateJobObject(self.0, 1) };
    }

    /// Clear `KILL_ON_JOB_CLOSE` so dropping the job handle no longer kills its
    /// members. Call this on the normal-completion path: on unix a successful
    /// command never triggers a process-group kill, so intentionally detached
    /// grandchildren (e.g. a browser launched by `future-cli browser start`)
    /// keep running. Without disarming, closing this job handle on drop would
    /// terminate that whole tree and the just-launched browser would die.
    /// Best-effort — a failure just leaves the kill-on-close behaviour intact.
    pub fn disarm(&self) {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = 0;
        // SAFETY: `info` is a correctly-sized, zero-initialized struct for this
        // class; clearing LimitFlags removes JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
        unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // Closing the last handle also triggers KILL_ON_JOB_CLOSE, covering the
        // drop-without-terminate path.
        unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod native_tests {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::path::PathBuf;

    use windows_sys::Win32::Storage::FileSystem::{FILE_ADD_FILE, FILE_WRITE_DATA};

    use super::acl::{ensure_write_deny, ensure_write_root};
    use super::audit::FrozenPath;
    use super::process::RestrictedChild;
    use super::token::{derive_capability_sid, RestrictedToken};
    use crate::sandbox::windows_capability::{CapabilityKind, CapabilityRecord};

    fn record(root: PathBuf, name: &str) -> CapabilityRecord {
        CapabilityRecord {
            name: name.to_owned(),
            kind: CapabilityKind::Policy,
            policy_fingerprint: "0".repeat(64),
            writable_root: root,
            request_id: None,
            approved_targets: vec![],
        }
    }

    fn token_or_skip(records: &[CapabilityRecord]) -> Option<RestrictedToken> {
        match RestrictedToken::from_capabilities(records) {
            Ok(token) => Some(token),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                eprintln!("SKIP: {error}");
                None
            }
            Err(error) => panic!("restricted token setup failed: {error}"),
        }
    }

    /// Real Windows AccessCheck matrix for the minimum restricting-SID set.
    /// It intentionally does not add Everyone or the logon SID.
    #[test]
    fn capability_sid_allows_only_root_and_deny_carveout_wins() {
        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().join("root");
        let carveout_path = root_path.join("protected");
        let external_path = directory.path().join("external");
        std::fs::create_dir_all(&carveout_path).unwrap();
        std::fs::create_dir_all(&external_path).unwrap();

        let record = record(root_path.clone(), "futureos.windows.w2-access-check-matrix");
        let sid = derive_capability_sid(&record.name).unwrap();
        let root = FrozenPath::open_local_ntfs(&root_path).unwrap();
        let carveout = FrozenPath::open_local_ntfs(&carveout_path).unwrap();
        let external = FrozenPath::open_local_ntfs(&external_path).unwrap();
        ensure_write_root(&root, &sid).unwrap();
        ensure_write_deny(&carveout, &sid).unwrap();
        // Re-applying must be idempotent rather than accumulating duplicate
        // FutureOS ACEs.
        ensure_write_root(&root, &sid).unwrap();
        ensure_write_deny(&carveout, &sid).unwrap();

        let Some(token) = token_or_skip(&[record]) else {
            return;
        };
        assert!(root.access_check(&token, FILE_ADD_FILE).unwrap());
        assert!(!external.access_check(&token, FILE_ADD_FILE).unwrap());
        assert!(!carveout.access_check(&token, FILE_WRITE_DATA).unwrap());
    }

    /// Native W3 smoke: CreateProcessAsUserW must preserve cwd/env/stdout and
    /// exit code while using the restricted token and no-breakaway Job.
    #[tokio::test]
    async fn restricted_powershell_spawn_captures_output_and_exit_code() {
        use tokio::io::AsyncReadExt;

        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().join("root");
        std::fs::create_dir_all(&root_path).unwrap();
        let record = record(root_path.clone(), "futureos.windows.w3-process-smoke");
        let sid = derive_capability_sid(&record.name).unwrap();
        let root = FrozenPath::open_local_ntfs(&root_path).unwrap();
        ensure_write_root(&root, &sid).unwrap();
        let Some(token) = token_or_skip(&[record]) else {
            return;
        };
        let args = vec![
            OsString::from("-NoProfile"),
            OsString::from("-NonInteractive"),
            OsString::from("-NoLogo"),
            OsString::from("-Command"),
            OsString::from("Write-Output \"$env:FUTUREOS_W3|$pwd\"; exit 7"),
        ];
        let environment = vec![(OsString::from("FUTUREOS_W3"), OsString::from("ready"))];
        let mut child = RestrictedChild::spawn(
            &token,
            OsStr::new("powershell.exe"),
            &args,
            &root_path,
            &environment,
        )
        .unwrap();
        assert_ne!(child.id(), 0);
        let mut stdout = tokio::fs::File::from_std(child.take_stdout().unwrap());
        let mut stderr = tokio::fs::File::from_std(child.take_stderr().unwrap());
        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).await.unwrap();
            bytes
        });

        assert_eq!(child.wait().await.unwrap(), 7);
        let stdout = String::from_utf8_lossy(&stdout_task.await.unwrap()).to_lowercase();
        let stderr = String::from_utf8_lossy(&stderr_task.await.unwrap()).to_string();
        assert!(
            stdout.contains("ready|") && stdout.contains("root"),
            "unexpected stdout: {stdout:?}"
        );
        assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr:?}");
    }

    #[tokio::test]
    async fn restricted_job_termination_stops_shell() {
        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().join("root");
        std::fs::create_dir_all(&root_path).unwrap();
        let record = record(root_path.clone(), "futureos.windows.w3-terminate-smoke");
        let sid = derive_capability_sid(&record.name).unwrap();
        let root = FrozenPath::open_local_ntfs(&root_path).unwrap();
        ensure_write_root(&root, &sid).unwrap();
        let Some(token) = token_or_skip(&[record]) else {
            return;
        };
        let args = vec![
            OsString::from("-NoProfile"),
            OsString::from("-NonInteractive"),
            OsString::from("-Command"),
            OsString::from("Start-Sleep -Seconds 30"),
        ];
        let mut child =
            RestrictedChild::spawn(&token, OsStr::new("powershell.exe"), &args, &root_path, &[])
                .unwrap();
        drop(child.take_stdout());
        drop(child.take_stderr());
        child.terminate();
        let exit = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .expect("terminated Job must finish promptly")
            .unwrap();
        assert_ne!(exit, 0);
    }
}
