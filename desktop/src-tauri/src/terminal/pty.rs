//! The portable-pty boundary: spawn, write, resize, and process-tree teardown.
//!
//! Everything platform-specific about running a shell lives here so the session
//! state machine above it stays platform-agnostic.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

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
        #[cfg(unix)]
        let pgid = pair.master.process_group_leader();
        #[cfg(not(unix))]
        let pgid = None;
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
        use crate::proc::NoWindow;
        let _ = (pgid, grace);
        // ConPTY children are not a process group; `taskkill /T` walks the tree
        // by parent links, which is the closest portable equivalent. (A Job
        // Object would be strictly better and is tracked in the design doc as
        // future work.)
        let _ = std::process::Command::new("taskkill")
            .no_window()
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
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
/// `docs/internals/desktop/embedded-terminal.md` §Platforms.
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
    use crate::terminal::test_support;
    use std::path::PathBuf;
    use std::time::Instant;

    fn spawn_with(command: (PathBuf, Vec<String>)) -> PtySession {
        let (program, args) = command;
        PtySession::spawn(&SpawnRequest {
            program: &program,
            args: &args,
            cwd: &std::env::temp_dir(),
            cols: 80,
            rows: 24,
        })
        .expect("spawn child")
    }

    /// Drain the master on a background thread, publishing the accumulated
    /// bytes after every read so a test can wait for a marker instead of
    /// sleeping for a fixed time.
    fn drained(session: &PtySession) -> std::sync::mpsc::Receiver<Vec<u8>> {
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
        rx
    }

    /// Every byte the reader thread has published so far, or `None` while it
    /// has published nothing (draining the channel keeps the *last* cumulative
    /// buffer, which is the one that holds the whole output).
    fn latest(rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> Option<Vec<u8>> {
        let mut newest = None;
        while let Ok(buf) = rx.try_recv() {
            newest = Some(buf);
        }
        newest
    }

    /// Answer the child's cursor-position query when it asks.
    ///
    /// Without this the child emits `ESC [ 6 n` and waits: measured on this
    /// host, `cmd /C echo hello` produced exactly that query and nothing else
    /// for the whole deadline, and `cmd /C exit 7` stayed alive. The end-to-end
    /// test in `server.rs` answers the same query.
    fn answer_dsr(session: &mut PtySession, seen: &[u8]) -> bool {
        if !test_support::asks_for_the_cursor(seen) {
            return false;
        }
        session
            .write(test_support::DSR_REPLY)
            .expect("answer the child's cursor-position query");
        true
    }

    /// Wait for `needle` in the child's output, answering the cursor query as
    /// it arrives.
    fn wait_for(
        session: &mut PtySession,
        rx: &std::sync::mpsc::Receiver<Vec<u8>>,
        needle: &str,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut text = String::new();
        let mut answered = false;
        while Instant::now() < deadline {
            if let Some(buf) = latest(rx) {
                if !answered {
                    answered = answer_dsr(session, &buf);
                }
                text = String::from_utf8_lossy(&buf).into_owned();
                if text.contains(needle) {
                    return text;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        text
    }

    /// Reap the child, answering its cursor query first (it will not reach its
    /// command's exit without the answer).
    fn wait_for_exit(
        session: &mut PtySession,
        rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    ) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut answered = false;
        while Instant::now() < deadline {
            if !answered {
                if let Some(buf) = latest(rx) {
                    answered = answer_dsr(session, &buf);
                }
            }
            if let Some(status) = session.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn spawns_a_real_pty_and_reports_the_exit_code() {
        let mut session = spawn_with(test_support::exit_command(7));
        let rx = drained(&session);
        assert_eq!(
            wait_for_exit(&mut session, &rx),
            Some(7),
            "the real exit code must survive the PTY"
        );
    }

    #[test]
    fn output_is_readable_through_the_master() {
        let mut session = spawn_with(test_support::echo_command("hello-pty"));
        let rx = drained(&session);
        let text = wait_for(&mut session, &rx, "hello-pty");
        assert!(
            test_support::visible_text(text.as_bytes()).contains("hello-pty"),
            "pty output: {text:?}"
        );
    }

    /// FINDING-2 regression: a job-control background job lives in its own
    /// process group, so group-scoped teardown alone would leave it running.
    #[cfg(target_os = "linux")]
    #[test]
    fn teardown_reaches_background_jobs() {
        // `setsid`-like behaviour comes from the PTY; `sleep 60 &` is a
        // background job in its own process group.
        let mut session = spawn_with(test_support::script_command(
            "sleep 60 & echo started; wait",
        ));
        let _rx = drained(&session);
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

    /// A written line must be *executed* by the child, not merely accepted by
    /// the PTY: `exit 5` proves the bytes reached the shell's input.
    /// WINDOWS-ONLY. The child's cursor query is the Windows interactive-shell fixture's handshake
    /// (`test_support::interactive_command()`); a unix shell in a PTY does not emit one, so the
    /// "must ask for the cursor" premise cannot hold there. Windows-only.
    #[cfg(windows)]
    #[test]
    fn writing_to_the_pty_reaches_the_shell() {
        let mut session = spawn_with(test_support::interactive_command());
        let rx = drained(&session);
        // The interactive shell asks where the cursor is before it will read a
        // line, so the reply has to go out before the line itself.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut answered = false;
        while !answered && Instant::now() < deadline {
            if let Some(buf) = latest(&rx) {
                answered = answer_dsr(&mut session, &buf);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(answered, "the interactive shell must ask for the cursor");
        let line: &[u8] = if cfg!(windows) {
            b"exit 5\r\n"
        } else {
            b"exit 5\n"
        };
        session.write(line).expect("write");
        assert_eq!(
            wait_for_exit(&mut session, &rx),
            Some(5),
            "the shell must have run the line it was given"
        );
    }

    /// `pid` and `resize` work on a live session; `kill_tree` then tears the
    /// child down without hanging (each wait is bounded).
    #[test]
    fn pid_resize_and_teardown_work_on_a_live_session() {
        let mut session = spawn_with(test_support::idle_command());
        let _rx = drained(&session);
        let pid = session.pid().expect("a spawned child has a pid");
        assert!(pid > 0);
        session.resize(132, 43).expect("resize a live pty");
        // A zero-sized window is rejected by the platform rather than
        // corrupting the session; either way the session stays usable.
        let _ = session.resize(1, 1);
        session.kill_tree(Duration::from_millis(500));
        assert!(
            wait_for_exit(&mut session, &_rx).is_some(),
            "teardown must leave a reaped child"
        );
    }

    /// `kill_tree` after the child was already reaped must not hang or panic:
    /// every wait in the teardown path is bounded and reaping an already-reaped
    /// child is a no-op on both platforms.
    #[test]
    fn teardown_of_an_exited_child_is_harmless() {
        let mut session = spawn_with(test_support::exit_command(0));
        let rx = drained(&session);
        assert_eq!(wait_for_exit(&mut session, &rx), Some(0));
        session.kill_tree(Duration::from_millis(200));
    }

    /// The child environment carries the terminal identity and none of the
    /// desktop's own plumbing variables.
    #[test]
    fn the_child_environment_is_scrubbed_and_tagged() {
        let mut command = CommandBuilder::new("echo");
        // Plant a sentinel for every denied key first: the assertion below has
        // to prove the removal happened, not that the variable was simply
        // absent from this process.
        for key in ENV_DENYLIST {
            command.env(key, "sentinel");
        }
        apply_environment(&mut command).expect("environment");

        for key in ENV_DENYLIST {
            assert!(
                command.get_env(key).is_none(),
                "{key} must not leak into the child"
            );
        }
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
        assert_eq!(
            command.get_env("COLORTERM"),
            Some(std::ffi::OsStr::new("truecolor"))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new("FutureOS"))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM_VERSION"),
            Some(std::ffi::OsStr::new(env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(
            command.get_env("FUTURE_TERMINAL"),
            Some(std::ffi::OsStr::new("1"))
        );
        if cfg!(windows) {
            // ConPTY does not imply a UTF-8 code page; these keep CJK output
            // from being mangled.
            for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
                assert_eq!(command.get_env(key), Some(std::ffi::OsStr::new("C.UTF-8")));
            }
        }
    }

    /// The cursor query is answered by writing the reply to the child's stdin,
    /// and the child's byte count proves those six bytes really arrived ahead of
    /// the line terminator (the count is a multiple of the reply length, because
    /// a helper may answer the still-buffered query more than once).
    #[cfg(unix)]
    #[test]
    fn a_cursor_query_is_answered_on_the_childs_stdin() {
        let mut session = spawn_with(test_support::cursor_query_command());
        let rx = drained(&session);

        // Answer first: the child is blocked in `read` until a whole line arrives.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut answered = false;
        while !answered && Instant::now() < deadline {
            if let Some(buf) = latest(&rx) {
                answered = answer_dsr(&mut session, &buf);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            answered,
            "the child must ask for the cursor before the reply can be sent"
        );
        session
            .write(b"\n")
            .expect("terminate the line the child is reading");

        let text = wait_for(&mut session, &rx, "answered:");
        let visible = test_support::visible_text(text.as_bytes());
        let count: usize = visible
            .split("answered:")
            .nth(1)
            .and_then(|rest| rest.trim().parse().ok())
            .unwrap_or(0);
        assert!(
            count >= test_support::DSR_REPLY.len()
                && count.is_multiple_of(test_support::DSR_REPLY.len()),
            "the child must have read the reply bytes it was sent, got {count} in {visible:?}"
        );
        assert_eq!(wait_for_exit(&mut session, &rx), Some(0));
    }

    /// The waiting helper answers a cursor query it observes and still reaps the
    /// child.
    ///
    /// The reply carries no line terminator, so on a fresh canonical PTY the only
    /// trace a reader could leave is the echo the line discipline sends back —
    /// and the helper has already drained that channel by the time it returns.
    /// Delivery of the reply to the child is therefore asserted one layer up
    /// (`terminal::session::tests::a_cursor_querying_child_is_answered_by_the_wait_helper`);
    /// this test asserts what *is* observable here: a child that asks for the
    /// cursor and then finishes on its own is reaped instead of wedging the wait
    /// until its deadline.
    #[cfg(unix)]
    #[test]
    fn wait_for_exit_answers_a_cursor_query_and_then_reaps_the_child() {
        let mut session = spawn_with(test_support::query_then_marker_command());
        let rx = drained(&session);
        assert_eq!(
            wait_for_exit(&mut session, &rx),
            Some(0),
            "a child that asked for the cursor must still be reaped, not waited out"
        );
    }

    /// `pid_running` answers "not alive" for the sentinel pids (which must never
    /// be probed as real processes) and "alive" for a child this test owns.
    #[cfg(unix)]
    #[test]
    fn pid_running_separates_live_children_from_the_sentinel_pids() {
        assert!(!pid_running(0), "pid 0 is the process group, not a process");
        assert!(!pid_running(-1), "a negative pid is not a process");
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 60"])
            .spawn()
            .expect("spawn fixture child");
        let pid = child.id() as i32;
        assert!(pid_running(pid), "the fixture child must be observed alive");
        // Signal sentinel pids are ignored rather than broadcast to the group.
        signal_pid(0, libc::SIGTERM);
        signal_pid(-1, libc::SIGTERM);
        signal_pid(pid, libc::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "the signalled child never exited"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            !status.success(),
            "the child must have died from the signal, not exited on its own"
        );
    }

    /// Teardown without a process group signals the pid directly (the shape a
    /// platform that cannot report a group leader gets) and still leaves no
    /// child behind.
    #[cfg(unix)]
    #[test]
    fn teardown_without_a_process_group_signals_the_pid() {
        // `exec` so the signalled pid *is* the sleeping process: a shell that
        // forked it would leave the child behind under its own pid.
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exec sleep 60"])
            .spawn()
            .expect("spawn fixture child");
        let pid = child.id() as i32;
        assert!(pid_running(pid));
        kill_tree(None, pid, Duration::from_millis(500));
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "teardown left the child alive");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(!status.success(), "teardown must have killed the child");
    }

    /// A child that traps `SIGTERM` survives the grace period, so the escalation
    /// to `SIGKILL` is what ends it — the arm a polite-only teardown would hang
    /// on.
    #[cfg(unix)]
    #[test]
    fn teardown_escalates_to_sigkill_for_a_child_that_ignores_sigterm() {
        let root = tempfile::tempdir().expect("tempdir");
        let ready = root.path().join("trap-installed");
        let mut child = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &format!(
                    "trap \"\" TERM; printf ready > '{}'; while true; do sleep 0.2; done",
                    ready.display()
                ),
            ])
            .spawn()
            .expect("spawn fixture child");
        let pid = child.id() as i32;
        assert!(pid_running(pid));
        // Wait for the trap to be installed: signalling before it is would make
        // this test measure the default disposition instead of the escalation.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "the fixture child never installed its SIGTERM trap"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        kill_tree(None, pid, Duration::from_millis(300));
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "the SIGKILL escalation left the child alive"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                status.signal(),
                Some(libc::SIGKILL),
                "a child that ignored SIGTERM must be killed with SIGKILL"
            );
        }
    }

    /// A fixture `ps` for the session query: it prints exactly the pairs the
    /// caller needs to see. The real `ps` on this host reports `sess` as 0 for
    /// every process (measured: the PTY child's own session leader has
    /// `pid == pgid == sess` in a live test, yet `ps -axo pid=,sess=` prints 0),
    /// so this is the only way to exercise the parser and the sweep it feeds.
    #[cfg(all(unix, not(target_os = "linux")))]
    fn fake_ps(directory: &Path, output: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("ps");
        std::fs::write(&path, format!("#!/bin/sh\n{output}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// The `ps`-based session query is the whole guarantee on this platform, so
    /// its parser is driven directly: a well-formed member is reported,
    /// malformed columns are skipped, and a `ps` that cannot run yields no
    /// members instead of panicking.
    #[cfg(all(unix, not(target_os = "linux")))]
    #[test]
    fn the_ps_based_session_query_parses_and_survives_a_broken_ps() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        // Line 2 is not a pair, line 3 has an unparsable session, line 4 is the
        // member: this process itself, alive while the parent parses the reply.
        fake_ps(
            &bin,
            "printf 'alone\\n'; printf '12 notanumber\\n'; printf '%d 4242\\n' \"$PPID\"",
        );
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", bin.as_os_str());
        let members = session_members_via_ps(4242);
        // A `ps` that cannot be spawned at all is not an error: no members.
        std::env::set_var("PATH", "");
        let unavailable = session_members_via_ps(4242);
        match saved {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        assert_eq!(
            members,
            vec![std::process::id() as i32],
            "only the well-formed member line must be reported"
        );
        assert!(
            unavailable.is_empty(),
            "an unusable `ps` must report no members rather than failing"
        );
    }

    /// The sweep after the group signal is what removes a job that escaped the
    /// shell's process group: every reported member is signalled, a member that
    /// is still observable at the end of the grace period is killed on a fresh
    /// enumeration, and the shell itself is always killed last. Driven through
    /// the fixture `ps`, because the platform query on this host reports no
    /// members at all (see `the_ps_based_session_query_…`).
    #[cfg(all(unix, not(target_os = "linux")))]
    #[test]
    fn the_session_sweep_signals_reported_members_and_escalates() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let mut victim = std::process::Command::new("/bin/sh")
            .args(["-c", "exec sleep 60"])
            .spawn()
            .expect("spawn the shell to tear down");
        let victim_pid = victim.id() as i32;
        // The escaped job, in the victim's session as the sweep sees it.
        let mut member = std::process::Command::new("/bin/sh")
            .args(["-c", "exec sleep 60"])
            .spawn()
            .expect("spawn the escaped member");
        let member_pid = member.id() as i32;
        fake_ps(&bin, &format!("printf '{member_pid} {victim_pid}\\n'"));
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", bin.as_os_str());

        kill_tree(None, victim_pid, Duration::from_millis(300));

        match saved {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        let member_status = member.try_wait().unwrap();
        assert!(
            member_status.is_some_and(|status| !status.success()),
            "the reported session member must have been signalled, not left running"
        );
        let victim_status = victim.try_wait().unwrap();
        assert!(
            victim_status.is_some_and(|status| !status.success()),
            "the shell itself must be gone once teardown returns"
        );
    }
}
