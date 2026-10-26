//! Starting a child that must outlive the process which started it.
//!
//! `future goal run` re-executes itself so the orchestrating agent never blocks
//! on a todo, and `supervisor watch` is a service that outlives every command
//! that started it. "Detached" is a platform promise, and on Windows it takes
//! more than a new process group:
//!
//! * **Windows** — `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` gives the child
//!   its own console and signal group, but it stays in the **Job Object** of
//!   whoever created it. A shell (or an agent's tool runner) that waits for its
//!   job to drain therefore never sees the spawned command finish: it waits
//!   until its own timeout, then kills the job — taking the worker that was
//!   dispatched a moment ago with it. `CREATE_BREAKAWAY_FROM_JOB` is what takes
//!   the child out of that job. It only works when the job grants
//!   `JOB_OBJECT_LIMIT_BREAKAWAY_OK`; a job that does not answers the attempt
//!   with `ERROR_ACCESS_DENIED`. So the flags are an ordered list of attempts,
//!   never one constant: the breakaway flag is preferred, and a refusal
//!   degrades to the plain detached flags instead of failing the dispatch.
//! * **Unix** — `process_group(0)` puts the child in its own process group, so
//!   a group-wide signal aimed at the starting shell (Ctrl-C, a runner's
//!   cleanup, `kill -- -<pgid>`) does not reach it. That is the same property
//!   the Windows breakaway flag buys, and it is measured in the test below: the
//!   parent's process group is SIGKILLed and the child must still be alive and
//!   in a group of its own. A full `setsid` is not needed for that promise —
//!   and would need `pre_exec`, which is `unsafe` and fails outright when the
//!   child is already a group leader.
//!
//! Nothing here creates, holds or inspects a Job Object itself: this module only
//! decides the flags a child is created with, and owns the one retry that makes
//! the decision portable.

use std::process::{Child, Command};

/// Windows: the child gets a console of its own instead of the parent's.
pub const DETACHED_PROCESS: u32 = 0x0000_0008;
/// Windows: the child is the root of a new process group (Ctrl-C does not fan
/// out to it).
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Windows: the child leaves the Job Object of the process that created it, so
/// the creator can finish while the child keeps running.
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// `CreateProcess` answers a breakaway attempt from a job that does not grant
/// `JOB_OBJECT_LIMIT_BREAKAWAY_OK` with this error.
const ERROR_ACCESS_DENIED: i32 = 5;

/// The `creation_flags` to try, preferred first: breakaway and plain detachment
/// differ by exactly the breakaway bit, so the fallback keeps every other
/// guarantee (own console, own process group).
pub fn creation_flag_attempts() -> [u32; 2] {
    let plain = CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS;
    [plain | CREATE_BREAKAWAY_FROM_JOB, plain]
}

/// Spawn `cmd` as a child that outlives the caller.
pub fn spawn(cmd: &mut Command) -> std::io::Result<Child> {
    platform_spawn(cmd)
}

#[cfg(unix)]
fn platform_spawn(cmd: &mut Command) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    cmd.spawn()
}

#[cfg(windows)]
fn platform_spawn(cmd: &mut Command) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    stop_inheriting_our_stdio();
    let (child, flags) = spawn_with_attempts(|flags| {
        cmd.creation_flags(flags);
        cmd.spawn()
    })?;
    if flags & CREATE_BREAKAWAY_FROM_JOB == 0 {
        // Said once: a job that forbids breakaway means the caller may still
        // take this child down with it, which is the failure this module exists
        // to avoid and cannot fix from here.
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            eprintln!(
                "detach: this process's job object does not allow \
                 CREATE_BREAKAWAY_FROM_JOB ({ERROR_ACCESS_DENIED}); the detached child stays \
                 inside it and may be terminated when the job is closed"
            );
        });
    }
    Ok(child)
}

#[cfg(not(any(unix, windows)))]
fn platform_spawn(cmd: &mut Command) -> std::io::Result<Child> {
    // No job objects and no process groups to claim here: the platform's plain
    // spawn is the honest answer rather than a claim this module cannot keep.
    cmd.spawn()
}

/// Windows: stop handing this process's own stdin/stdout/stderr to children.
///
/// Breakaway is not sufficient on its own to make a dispatch return. A child
/// created with `bInheritHandles = TRUE` — which is what any redirected stdio
/// needs — inherits **every** inheritable handle of its parent, including the
/// pipe a caller put around this command. The dispatched worker then holds that
/// pipe open for its whole life, so a caller that reads output to EOF waits
/// until its own timeout even though the command printed and exited long
/// before. Clearing the inherit bit on our own three std handles removes the
/// handle that leak actually runs through.
///
/// What this cannot do is chase copies further up the chain: when the caller is
/// an intermediate shell that keeps a second inheritable copy of ITS caller's
/// pipe, the worker still inherits that copy. Closing the leak for good needs
/// `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (a process created through
/// `CreateProcessW` with an allow-list of exactly its three stdio handles);
/// that is out of scope here, and the live check in the handoff reports the CLI
/// process's own exit time for that reason.
#[cfg(windows)]
fn stop_inheriting_our_stdio() {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: both calls only read the pseudo-handle this process already
        // owns and flip a flag on it; a failure (a closed std handle) is ignored
        // because there is nothing then to stop inheriting.
        let handle: HANDLE = unsafe { GetStdHandle(which) };
        if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            continue;
        }
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    }
}

/// Try the preferred flags, then the degraded set when the job refuses
/// breakaway. Returns the child and the flags it was actually created with, so
/// the caller can report a degraded spawn instead of silently assuming the
/// preferred promise.
///
/// Only `ERROR_ACCESS_DENIED` is retried: it is the one answer that means "this
/// job will not allow the flag". Any other failure is a real spawn failure and
/// is reported as-is instead of being papered over with a second attempt.
#[cfg(any(windows, test))]
fn spawn_with_attempts<F>(mut attempt: F) -> std::io::Result<(Child, u32)>
where
    F: FnMut(u32) -> std::io::Result<Child>,
{
    let [preferred, degraded] = creation_flag_attempts();
    match attempt(preferred) {
        Ok(child) => Ok((child, preferred)),
        Err(error) if refuses_breakaway(&error) => attempt(degraded).map(|child| (child, degraded)),
        Err(error) => Err(error),
    }
}

/// Whether a spawn failure is the job object refusing breakaway (as opposed to
/// any other reason a child could not be created).
#[cfg(any(windows, test))]
fn refuses_breakaway(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// The production flag set: the breakaway bit is present in the attempt that
    /// is tried first, the fallback differs only by that bit, and both keep the
    /// detachment promises (own console, own process group).
    #[test]
    fn the_preferred_flags_break_away_and_the_fallback_only_drops_that_bit() {
        let [preferred, degraded] = creation_flag_attempts();
        assert_eq!(
            preferred & CREATE_BREAKAWAY_FROM_JOB,
            CREATE_BREAKAWAY_FROM_JOB,
            "a detached child must leave the caller's job: {preferred:#x}"
        );
        assert_eq!(
            degraded & CREATE_BREAKAWAY_FROM_JOB,
            0,
            "the fallback exists precisely because the job refused that flag: {degraded:#x}"
        );
        assert_eq!(
            preferred ^ degraded,
            CREATE_BREAKAWAY_FROM_JOB,
            "the fallback must change nothing except the breakaway bit"
        );
        for flags in [preferred, degraded] {
            assert_eq!(flags & CREATE_NEW_PROCESS_GROUP, CREATE_NEW_PROCESS_GROUP);
            assert_eq!(flags & DETACHED_PROCESS, DETACHED_PROCESS);
        }
    }

    /// A job that forbids breakaway must degrade, not fail the dispatch. The
    /// fixture is the refusal itself (`ERROR_ACCESS_DENIED` from the preferred
    /// attempt) rather than a real Job Object, so the fallback is exercised on
    /// every platform. The Windows-only `a_real_job_that_forbids_breakaway_...`
    /// test below proves the error code is the one a real job returns.
    #[test]
    fn a_refused_breakaway_retries_once_without_that_flag() {
        let [preferred, degraded] = creation_flag_attempts();
        let mut tried = Vec::new();
        let mut cmd = noop_command();
        let (mut child, used) = spawn_with_attempts(|flags| {
            tried.push(flags);
            if flags == preferred {
                return Err(std::io::Error::from_raw_os_error(ERROR_ACCESS_DENIED));
            }
            cmd.spawn()
        })
        .expect("a refused breakaway must degrade instead of failing");
        let _ = child.wait();
        assert_eq!(
            tried,
            vec![preferred, degraded],
            "exactly one retry, with the plain detached flags"
        );
        assert_eq!(used, degraded);
    }

    /// Any other failure is the real answer: no second attempt, and the error is
    /// the caller's to report.
    #[test]
    fn a_failure_that_is_not_a_refused_breakaway_is_not_retried() {
        let mut tries = 0usize;
        let error = spawn_with_attempts(|_flags| {
            tries += 1;
            Err(std::io::Error::from_raw_os_error(2))
        })
        .expect_err("a non-job failure must be reported, not hidden");
        assert_eq!(tries, 1, "only ERROR_ACCESS_DENIED earns a retry");
        assert_eq!(error.raw_os_error(), Some(2));
    }

    /// A command that exits immediately, for the flag-attempt tests.
    fn noop_command() -> Command {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("cmd");
            cmd.args(["/c", "exit"]);
            cmd
        }
        #[cfg(not(windows))]
        {
            Command::new("true")
        }
    }

    // ── Unix: the property the Windows breakaway flag buys ─────────────────

    /// Set on the helper process below to name the file it reports into.
    #[cfg(unix)]
    const HELPER_OUT_ENV: &str = "FUTURE_GOAL_DETACH_TEST_OUT";

    /// The parent half of `a_detached_child_survives_its_parents_process_group_
    /// being_killed`: spawn a long-lived child through this module, report both
    /// pids, then wait to be killed. A no-op unless the driver asks for it.
    #[cfg(unix)]
    #[test]
    fn detached_parent_helper() {
        let Ok(out) = std::env::var(HELPER_OUT_ENV) else {
            return;
        };
        let mut cmd = Command::new("sleep");
        cmd.arg("120")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = spawn(&mut cmd).expect("spawn the detached child");
        std::fs::write(&out, format!("{},{}", child.id(), std::process::id()))
            .expect("report the pids");
        std::thread::sleep(std::time::Duration::from_secs(120));
    }

    /// A detached child must survive the death of its parent's process group —
    /// the Unix reading of "the caller's job killed the worker", and the reason
    /// `process_group(0)` is not optional. A child left in the parent's group is
    /// signalled by the group kill below and this test fails; the assertion that
    /// the child is alive *and* in a different group is what proves the child
    /// outlived the kill because of the group change, not because the kill
    /// missed it.
    #[cfg(unix)]
    #[test]
    fn a_detached_child_survives_its_parents_process_group_being_killed() {
        use std::os::unix::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("child.pid");
        let exe = std::env::current_exe().unwrap();
        let mut helper_cmd = Command::new(&exe);
        helper_cmd
            .args(["detach::tests::detached_parent_helper", "--exact"])
            .env(HELPER_OUT_ENV, &out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The helper gets a process group of its own: the group kill below must
        // hit the helper alone. Without this it would inherit the harness's
        // group and the kill would take the whole test run down with it.
        helper_cmd.process_group(0);
        let mut helper = helper_cmd.spawn().expect("run the parent helper");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let reported = loop {
            if let Ok(text) = std::fs::read_to_string(&out) {
                let mut parts = text.split(',');
                if let (Some(child), Some(parent)) = (parts.next(), parts.next()) {
                    if let (Ok(child), Ok(parent)) =
                        (child.trim().parse::<i32>(), parent.trim().parse())
                    {
                        break (child, parent);
                    }
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the parent helper never reported its child's pid"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
            if let Ok(Some(status)) = helper.try_wait() {
                panic!("the parent helper exited before reporting a pid: {status}");
            }
        };
        let (child_pid, parent_pid) = reported;

        // Signal the parent's whole group, the way a shell or a CI runner ends
        // its children. The helper is killed here; the detached child is not.
        let parent_group = unsafe { libc::getpgid(parent_pid) };
        assert!(parent_group > 0, "the helper must be in a process group");
        assert_eq!(
            parent_group, parent_pid,
            "the fixture spawns the helper as its own group leader, so the group \
             kill below hits the helper alone"
        );
        assert_eq!(
            unsafe { libc::kill(-parent_group, libc::SIGKILL) },
            0,
            "the fixture must be able to kill the parent's group"
        );
        let _ = helper.wait();

        let child_group = unsafe { libc::getpgid(child_pid) };
        assert!(
            child_group > 0,
            "the detached child ({child_pid}) is gone: it did not survive the group kill"
        );
        assert_ne!(
            child_group, parent_group,
            "the child must be in a process group of its own"
        );
        assert_eq!(
            unsafe { libc::kill(child_pid, libc::SIGKILL) },
            0,
            "cleanup: the surviving child must still be signallable"
        );
    }

    // ── Windows: the real refusal a Job Object answers with ────────────────

    #[cfg(windows)]
    const JOB_HELPER_ENV: &str = "FUTURE_GOAL_DETACH_JOB_HELPER";

    /// Run the platform spawn inside a Job Object that does not grant
    /// `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, which is the state every caller's shell
    /// is in by default. A no-op unless the driver below asks for it: joining a
    /// job is process-wide, so it must never happen in the test harness itself.
    #[cfg(windows)]
    #[test]
    fn detached_job_helper() {
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        if std::env::var(JOB_HELPER_ENV).is_err() {
            return;
        }
        // SAFETY: a fresh unnamed job with default limits, and the pseudo-handle
        // of this process; the job handle is closed at the end of the test.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        assert!(!job.is_null(), "create a job object");
        // SAFETY: both handles belong to this process and outlive the call.
        let assigned = unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) };
        assert!(assigned != 0, "join the job that forbids breakaway");

        // The fixture is real only if the same job REFUSES an explicit
        // breakaway: assert the error code this module degrades on.
        let mut raw = noop_command();
        {
            use std::os::windows::process::CommandExt;
            raw.creation_flags(creation_flag_attempts()[0]);
        }
        let refusal = raw
            .spawn()
            .expect_err("a default job must refuse CREATE_BREAKAWAY_FROM_JOB");
        assert_eq!(
            refusal.raw_os_error(),
            Some(ERROR_ACCESS_DENIED),
            "the refusal must be the one this module retries on: {refusal}"
        );

        // And the seam still starts the child, degraded rather than at all.
        let mut cmd = noop_command();
        let mut child = spawn(&mut cmd).expect("degrade instead of failing the dispatch");
        let _ = child.wait();
        // SAFETY: `job` is a handle this test created and never handed elsewhere.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(job) };
    }

    /// Drive `detached_job_helper` in a child process: joining a job is
    /// process-wide, so it cannot happen in the harness that runs every other
    /// test. The helper asserts the refusal and the degraded spawn itself; this
    /// driver only fails when the helper does.
    #[cfg(windows)]
    #[test]
    fn a_real_job_that_forbids_breakaway_degrades_instead_of_failing() {
        let exe = std::env::current_exe().unwrap();
        let output = Command::new(&exe)
            .args(["detach::tests::detached_job_helper", "--exact"])
            .env(JOB_HELPER_ENV, "1")
            .stdin(Stdio::null())
            .output()
            .expect("run the job helper");
        assert!(
            output.status.success(),
            "the job helper failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "the helper must have run its assertions: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}
