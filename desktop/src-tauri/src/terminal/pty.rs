//! The portable-pty boundary: spawn, write, resize, and process-tree teardown.
//!
//! Everything platform-specific about running a shell lives here so the session
//! state machine above it stays platform-agnostic.

use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Environment variables the app injects for its own plumbing and must not
/// leak into a user shell. Kept as an explicit list: a blanket `FUTURE_*`
/// filter would also drop settings the user's own profile legitimately sets.
const ENV_DENYLIST: &[&str] = &[
    // The desktop process points itself at its supervised agent over gRPC.
    "FUTURE_AGENT_GRPC_ADDR",
    // Defensive: the terminal server's own secret is never exported, but a
    // future refactor must not be able to leak it through the child env.
    "FUTURE_TERMINAL_TOKEN",
];

pub struct SpawnRequest<'a> {
    pub program: &'a Path,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub cols: u16,
    pub rows: u16,
}

/// A live PTY child plus the master side used to talk to it.
pub struct PtySession {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    /// Process-group id of the spawned shell (the shell is made a session
    /// leader by portable-pty, so this equals its pid on unix).
    pgid: Option<i32>,
    pid: Option<u32>,
}

impl PtySession {
    /// Spawn `program` on a fresh PTY with the user's environment.
    pub fn spawn(request: &SpawnRequest<'_>) -> Result<Self, String> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: request.rows,
                cols: request.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("SPAWN_FAILED: openpty: {error}"))?;

        let mut command = CommandBuilder::new(request.program.as_os_str());
        command.args(request.args.iter().map(|arg| arg.as_str()));
        command.cwd(request.cwd);
        apply_environment(&mut command)?;

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("SPAWN_FAILED: {}: {error}", request.program.display()))?;
        // The slave end belongs to the child once spawned. Dropping our copy in
        // the parent is what lets the reader observe EOF when the shell exits —
        // keeping it open would make every session look immortal.
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("SPAWN_FAILED: pty writer: {error}"))?;
        let pgid = pair.master.process_group_leader();
        let pid = child.process_id();
        Ok(Self {
            child,
            master: pair.master,
            writer,
            pgid,
            pid,
        })
    }

    /// A blocking reader for the PTY output. One per session, consumed by the
    /// session's reader thread.
    pub fn reader(&self) -> Result<Box<dyn Read + Send>, String> {
        self.master
            .try_clone_reader()
            .map_err(|error| format!("PTY_IO_FAILED: reader: {error}"))
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("PTY_IO_FAILED: resize: {error}"))
    }

    /// Non-blocking exit check; `Some` once the shell has been reaped.
    pub fn try_wait(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            _ => None,
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Tear down the whole tree, not just the shell process.
    ///
    /// A shell with job control puts background jobs in their own process
    /// groups, so `killpg(shell)` alone leaves `sleep 300 &` running — verified
    /// in the T02 spike (FINDING-2). Session-scoped enumeration is therefore
    /// part of the contract, not an optimisation.
    pub fn kill_tree(&mut self, grace: Duration) {
        if let Some(pid) = self.pid {
            kill_tree(self.pgid, pid as i32, grace);
        }
        // Reap our own child last so its exit does not leave a zombie behind.
        let _ = self.child.kill();
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Best-effort: never hang process teardown on a wedged child.
        let _ = self.child.kill();
    }
}

/// Build the child environment: the user's real environment plus the few
/// variables a terminal must set, minus the app's own plumbing.
fn apply_environment(command: &mut CommandBuilder) -> Result<(), String> {
    for key in ENV_DENYLIST {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "FutureOS");
    command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    // Lets a shell profile / prompt detect it runs inside a FutureOS tab
    // (opencode sets `OPENCODE_TERMINAL` for the same reason).
    command.env("FUTURE_TERMINAL", "1");
    if cfg!(windows) {
        // ConPTY does not imply a UTF-8 code page; without these the shell
        // mangles non-ASCII input and output.
        command.env("LC_ALL", "C.UTF-8");
        command.env("LC_CTYPE", "C.UTF-8");
        command.env("LANG", "C.UTF-8");
    }
    Ok(())
}

/// Terminate a shell and everything it started.
///
/// Order: the shell's process group gets SIGTERM, then SIGKILL; whatever is
/// still alive is swept by session membership. Every wait is bounded — a wedged
/// child must not be able to hang the desktop's shutdown path.
pub fn kill_tree(pgid: Option<i32>, pid: i32, grace: Duration) {
    #[cfg(unix)]
    {
        if let Some(pgid) = pgid.filter(|value| *value > 0) {
            signal_group(pgid, libc::SIGTERM);
        } else {
            signal_pid(pid, libc::SIGTERM);
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if !pid_running(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        if pid_running(pid) {
            if let Some(pgid) = pgid.filter(|value| *value > 0) {
                signal_group(pgid, libc::SIGKILL);
            } else {
                signal_pid(pid, libc::SIGKILL);
            }
        }

        // Background jobs escaped the shell's process group: sweep the session.
        let members = session_members(pid);
        for member in &members {
            signal_pid(*member, libc::SIGTERM);
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if members.iter().all(|member| !pid_running(*member)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        for member in session_members(pid) {
            signal_pid(member, libc::SIGKILL);
        }
        signal_pid(pid, libc::SIGKILL);
    }

    #[cfg(windows)]
    {
        let _ = grace;
        // ConPTY children are not a process group; `taskkill /T` walks the tree
        // by parent links, which is the closest portable equivalent. (A Job
        // Object would be strictly better and is tracked in the design doc as
        // future work.)
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pgid, pid, grace);
    }
}

/// True while the process exists and is not a zombie.
///
/// `kill(pid, 0)` alone would report a zombie as alive (its pid is still
/// reserved until reaped), which would make every grace period run to its
/// deadline. Unix-only: the Windows teardown path hands the tree to `taskkill`
/// and never inspects a pid.
#[cfg(unix)]
pub fn pid_running(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Some(state) = linux_state(pid) {
        return state != 'Z' && state != 'X';
    }
    // SAFETY: signal 0 performs the permission/existence check only.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(target_os = "linux")]
fn linux_state(pid: i32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The comm field is parenthesised and may itself contain spaces or
    // parentheses; state is the first token after the LAST ')'.
    let index = stat.rfind(')')?;
    stat[index + 1..].split_whitespace().next()?.chars().next()
}

#[cfg(unix)]
fn signal_pid(pid: i32, signal: i32) {
    if pid <= 0 {
        return;
    }
    // SAFETY: sending a signal to a pid we spawned; failure is not fatal.
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

#[cfg(unix)]
fn signal_group(pgid: i32, signal: i32) {
    // SAFETY: negative pid targets the process group.
    unsafe {
        libc::killpg(pgid as libc::pid_t, signal);
    }
}

/// Live pids in the same session as `sid`.
///
/// Linux reads `/proc`; other unix builds fall back to a `ps` scan for the
/// session column, and to a parent-chain walk when even that is unavailable.
/// macOS/Windows teardown is explicitly unverified — see
/// `desktop/DEV_MD/embedded-terminal.md` §Platforms.
#[cfg(unix)]
pub fn session_members(sid: i32) -> Vec<i32> {
    #[cfg(target_os = "linux")]
    {
        let mut members = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                    continue;
                };
                if !pid_running(pid) {
                    continue;
                }
                // SAFETY: getsid only reads kernel state for an existing pid.
                let session = unsafe { libc::getsid(pid as libc::pid_t) };
                if session == sid as libc::pid_t {
                    members.push(pid);
                }
            }
        }
        members.sort_unstable();
        members
    }

    #[cfg(not(target_os = "linux"))]
    {
        session_members_via_ps(sid)
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn session_members_via_ps(sid: i32) -> Vec<i32> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pid=,sess="])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut members = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(session)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(pid), Ok(session)) = (pid.parse::<i32>(), session.parse::<i32>()) else {
            continue;
        };
        if pid != sid && session == sid && pid_running(pid) {
            members.push(pid);
        }
    }
    members
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn shell_path() -> PathBuf {
        PathBuf::from("/bin/sh")
    }

    fn spawn_sh(script: &str) -> PtySession {
        let args = vec!["-c".to_string(), script.to_string()];
        PtySession::spawn(&SpawnRequest {
            program: &shell_path(),
            args: &args,
            cwd: &std::env::temp_dir(),
            cols: 80,
            rows: 24,
        })
        .expect("spawn /bin/sh")
    }

    #[cfg(unix)]
    #[test]
    fn spawns_a_real_pty_and_reports_exit_code() {
        let mut session = spawn_sh("exit 7");
        let mut reader = session.reader().expect("reader");
        std::thread::spawn(move || {
            let mut sink = [0_u8; 1024];
            while matches!(reader.read(&mut sink), Ok(n) if n > 0) {}
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut code = None;
        while Instant::now() < deadline {
            if let Some(status) = session.try_wait() {
                code = Some(status);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(code, Some(7), "the real exit code must survive the PTY");
    }

    #[cfg(unix)]
    #[test]
    fn output_is_readable_through_the_master() {
        let session = spawn_sh("printf 'hello-pty\\n'");
        let mut reader = session.reader().expect("reader");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0_u8; 256];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let _ = tx.send(buf.clone());
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut text = String::new();
        while Instant::now() < deadline {
            if let Ok(buf) = rx.try_recv() {
                text = String::from_utf8_lossy(&buf).into_owned();
                if text.contains("hello-pty") {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(text.contains("hello-pty"), "pty output: {text:?}");
    }

    /// FINDING-2 regression: a job-control background job lives in its own
    /// process group, so group-scoped teardown alone would leave it running.
    #[cfg(target_os = "linux")]
    #[test]
    fn teardown_reaches_background_jobs() {
        // `setsid`-like behaviour comes from the PTY; `sleep 60 &` is a
        // background job in its own process group.
        let mut session = spawn_sh("sleep 60 & echo started; wait");
        let mut reader = session.reader().expect("reader");
        std::thread::spawn(move || {
            let mut sink = [0_u8; 1024];
            while matches!(reader.read(&mut sink), Ok(n) if n > 0) {}
        });
        let pid = session.pid().expect("pid");
        // Give the shell a moment to fork the background job.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut members = Vec::new();
        while Instant::now() < deadline {
            members = session_members(pid as i32);
            if members.len() > 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            members.len() > 1,
            "the background job must be observable as a session member: {members:?}"
        );

        session.kill_tree(Duration::from_millis(300));
        assert!(
            session_members(pid as i32).iter().all(|m| !pid_running(*m)),
            "every session member must be gone after teardown"
        );
    }

    #[cfg(unix)]
    #[test]
    fn writing_to_the_pty_reaches_the_shell() {
        // `cat` echoes stdin back; write a marker and read it.
        let mut session = spawn_sh("cat");
        let mut reader = session.reader().expect("reader");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0_u8; 256];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let _ = tx.send(buf.clone());
            }
        });
        session.write(b"marker-42\n").expect("write");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut text = String::new();
        while Instant::now() < deadline {
            if let Ok(buf) = rx.try_recv() {
                text = String::from_utf8_lossy(&buf).into_owned();
                if text.contains("marker-42") {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        session.kill_tree(Duration::from_millis(200));
        assert!(text.contains("marker-42"), "echoed output: {text:?}");
    }
}
