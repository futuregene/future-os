//! Windows sandbox executor (SANDBOX_PLAN §11), `#[cfg(windows)]` only.
//!
//! W1b lands in slices so each can be compiled/verified on Windows:
//!   1. **Job Object** (this file so far) — process-tree teardown, the Windows
//!      analog of the unix process-group kill. Needed for every bash run, not
//!      just sandboxed ones, so it ships before the enforcement layer.
//!   2. restricted token ({Users, RESTRICTED}) — later slice.
//!   3. ACE guard from [`super::windows_plan::WindowsSandboxPlan`] — later slice.
//!   4. launch glue → `build_command` — later slice.

use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// A job object configured to kill every member process when the handle closes.
/// Assign a freshly-spawned bash PID; dropping the job (or calling
/// [`Job::terminate`]) then kills bash and all of its descendants as a tree.
pub struct Job(HANDLE);

// A raw job HANDLE is safe to move/hand across the spawn future.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// Create an anonymous `KILL_ON_JOB_CLOSE` job object.
    pub fn create() -> io::Result<Self> {
        // SAFETY: null attributes/name request an anonymous, default-secured job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job(handle);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
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
        let ok = unsafe { AssignProcessToJobObject(self.0, process) };
        unsafe { CloseHandle(process) };
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
}

impl Drop for Job {
    fn drop(&mut self) {
        // Closing the last handle also triggers KILL_ON_JOB_CLOSE, covering the
        // drop-without-terminate path.
        unsafe { CloseHandle(self.0) };
    }
}
