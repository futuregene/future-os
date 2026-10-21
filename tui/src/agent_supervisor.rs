//! TUI-owned Agent sidecar lifecycle.
//!
//! A TUI validates its configured Agent before sending business requests. Only
//! an absent default local endpoint and an available instance lock authorize
//! the unified `future agent` sidecar, and that sidecar is always this
//! process' own executable re-run with `agent --grpc-addr auto`: the
//! standalone `future-tui` binary refuses to launch one instead of guessing at
//! sibling or `PATH` programs.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use future_rpc::supervision::{
    probe_local_agent, restart_delay, Attached, LocalAgentState, RestartBudget, WAIT_FOR_OWNER,
    WAIT_NOTICE_AFTER, WAIT_PROBE_INTERVAL,
};

/// Per-attempt probe budget inside the supervision loop. Local IPC answers in
/// milliseconds when it answers at all.
const PROBE_CONNECT: Duration = Duration::from_millis(400);
const PROBE_HANDSHAKE: Duration = Duration::from_secs(2);

/// How often an attached Agent is re-probed. Long enough to stay out of the
/// way, short enough that a death is noticed before the user blames the TUI.
const PROBE_INTERVAL: Duration = Duration::from_secs(3);

fn executable_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Arguments the sidecar is spawned with: the unified binary's own `agent`
/// subcommand, pinned to `--grpc-addr auto` so the child binds the
/// instance-local endpoint the TUI will probe, never a caller-chosen address.
fn sidecar_args() -> Vec<String> {
    vec![
        "agent".to_string(),
        "--grpc-addr".to_string(),
        "auto".to_string(),
    ]
}

/// Whether `exe` may be re-executed as the `future agent` sidecar.
///
/// Only the unified `future` binary implements the `agent` subcommand. The
/// standalone `future-tui` binary spawned with these arguments would start
/// *another TUI* instead of an agent, and no other program interprets them
/// either — both are refused here, before a child can be forked. The check
/// runs only once a launch is actually needed, so attaching to an agent that
/// is already running works from any binary.
fn check_sidecar_program(exe: &Path) -> Result<(), String> {
    if executable_name(exe) == "future" {
        return Ok(());
    }
    Err(
        "the standalone future-tui binary cannot start an agent sidecar; \
         run `future tui` (unified binary) or start an Agent first"
            .to_string(),
    )
}

/// The program the sidecar is spawned from: this process' own executable.
fn resolve_sidecar_program() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("cannot locate the current executable: {error}"))
}

pub struct OwnedAgent {
    child: Child,
}

impl Drop for OwnedAgent {
    fn drop(&mut self) {
        kill_and_reap(&mut self.child);
    }
}

/// How long a sidecar that refuses to die gets before the TUI stops waiting for
/// it. SIGKILL lands in milliseconds; a process stuck in an uninterruptible
/// syscall is the case this bound covers.
const REAP_GRACE: Duration = Duration::from_millis(500);

/// End a sidecar child without ever blocking on its exit.
///
/// `Child::wait` is unbounded, and the launch reaches this code precisely when
/// the sidecar is unhealthy — a wedged child would then turn "the agent did not
/// start" into "the terminal never came up". Leaving one child unreaped costs a
/// process slot; a blocking wait costs the whole launch (and, in `Drop`, the
/// whole exit).
fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let deadline = std::time::Instant::now() + REAP_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) if std::time::Instant::now() >= deadline => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// How long a freshly spawned sidecar is given to answer the readiness probe
/// before the launch kills it and reports failure.
///
/// A *product* budget: it is how long a user waits for a cold sidecar before the
/// TUI says no agent could be started, so it is stated once, here. The launch's
/// other fixed costs are the per-attempt connect and handshake timeouts below.
const LAUNCH_PROBE_WINDOW: Duration = Duration::from_millis(5_000);

/// Interval between readiness probes inside [`LAUNCH_PROBE_WINDOW`].
const LAUNCH_PROBE_INTERVAL: Duration = Duration::from_millis(125);

/// Return `None` when an existing Agent was found, or an owned child when the
/// TUI had to launch one. The child is terminated when the returned guard is
/// dropped.
pub async fn ensure_agent_running(configured: &str) -> Result<Option<OwnedAgent>, String> {
    let program = resolve_sidecar_program()?;
    ensure_agent_running_within(configured, &program, LAUNCH_PROBE_WINDOW).await
}

/// [`ensure_agent_running`] with the sidecar program and the probe window
/// supplied by the caller.
///
/// `program` is the seam the launch fixtures inject a stand-in through, and it
/// must be named like the unified `future` binary — the guard before the spawn
/// refuses anything else. The window is the product budget above, and a caller
/// that has to fork/exec a stand-in passes its own instead of asserting how
/// quickly the host schedules that child: measured on a loaded host, a sidecar
/// that takes longer than the default window to become observable loses the
/// race, and the launch then reports `no Future Agent was reachable` - which is
/// a statement about the host, not about the code under test (see
/// `docs/testing/flaky-tui.md` §7.1).
pub async fn ensure_agent_running_within(
    configured: &str,
    program: &Path,
    probe_window: Duration,
) -> Result<Option<OwnedAgent>, String> {
    let permit = match future_rpc::connection::select_agent(
        configured,
        future_rpc::connection::EXPECTED_AGENT_VERSION,
        Duration::from_millis(400),
        Duration::from_secs(2),
    )
    .await
    .map_err(|error| error.to_string())?
    {
        future_rpc::connection::AgentSelection::External(_) => return Ok(None),
        future_rpc::connection::AgentSelection::LocalStart(permit) => permit,
    };
    permit.recheck().map_err(|error| error.to_string())?;

    if cfg!(test) {
        return Err("agent sidecar startup is disabled in unit tests".to_string());
    }

    // Only reached once a launch is actually needed: attaching to an agent
    // that is already running must work regardless of which binary is running.
    check_sidecar_program(program)?;

    let mut command = Command::new(program);
    command
        .args(sidecar_args())
        .stdin(Stdio::null())
        // Piped so the failure report below can quote the sidecar's own words
        // instead of only its exit status. The drain threads keep the pipes
        // empty while the sidecar lives, which is also what keeps a chatty
        // child from ever blocking on a full pipe.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let failure = format!("{} agent: {error}", program.display());
            return Err(sidecar_launch_failed(failure));
        }
    };
    let (stdout_tail, stderr_tail, stdout_done, stderr_done) = spawn_output_drains(&mut child);
    let attempts = (probe_window.as_millis() / LAUNCH_PROBE_INTERVAL.as_millis()).max(1) as usize;
    // The give-up reason unless the probe loop observes something more specific
    // first: the window ran out without the sidecar ever answering. A sidecar
    // that spawned but was never accepted must not vanish from the report just
    // because it outlived the window.
    let mut failure = format!(
        "{} agent did not become ready within {probe_window:?}",
        program.display()
    );
    for _ in 0..attempts {
        match future_rpc::connection::connect_verified(
            configured,
            future_rpc::connection::EXPECTED_AGENT_VERSION,
            Duration::from_millis(400),
            Duration::from_secs(2),
        )
        .await
        {
            Ok(_) => {
                // The drain threads stay behind on purpose: they took the pipes
                // with them, and when this guard is dropped it kills the child,
                // the pipes close, and each thread reads EOF and ends — filling
                // tail buffers nobody reads, so nothing here waits on them.
                return Ok(Some(OwnedAgent { child }));
            }
            Err(
                future_rpc::connection::ConnectionError::Transport(_)
                | future_rpc::connection::ConnectionError::Unverified(_),
            ) => {}
            Err(error) => {
                kill_and_reap(&mut child);
                let mut failure = error.to_string();
                attach_last_output(
                    &mut failure,
                    tail_after_exit(&stdout_done, &stderr_done, &stdout_tail, &stderr_tail),
                );
                return Err(failure);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                failure = format!("{} exited with {status}", program.display());
                break;
            }
            Ok(None) => tokio::time::sleep(LAUNCH_PROBE_INTERVAL).await,
            Err(error) => {
                failure = format!("{}: {error}", program.display());
                break;
            }
        }
    }
    kill_and_reap(&mut child);
    attach_last_output(
        &mut failure,
        tail_after_exit(&stdout_done, &stderr_done, &stdout_tail, &stderr_tail),
    );
    Err(sidecar_launch_failed(failure))
}

/// [`ensure_agent_running`] for a launch that must not fail on a busy lock.
///
/// Losing the instance-lock race is not a reason to refuse to start: another
/// process is bringing the Agent up, and the only thing this client has to do
/// is wait for it. Waiting is bounded ([`WAIT_FOR_OWNER`]) and says so while it
/// happens — a client that waits silently and then fails is worse than one that
/// fails immediately, because the user cannot tell the two apart.
///
/// Everything that is *not* a held lock (an unreachable explicit endpoint, a
/// version mismatch, a stop in progress) is still reported at once: no amount of
/// waiting repairs those.
pub async fn ensure_agent_running_waiting(configured: &str) -> Result<Option<OwnedAgent>, String> {
    let started = Instant::now();
    let mut last_notice: Option<Instant> = None;
    loop {
        match probe_local_agent(configured, PROBE_CONNECT, PROBE_HANDSHAKE).await {
            LocalAgentState::Busy(detail) => {
                let waited = started.elapsed();
                if waited >= WAIT_FOR_OWNER {
                    return Err(format!(
                        "another process has held this data root's Agent lock for {WAIT_FOR_OWNER:?} \
                         without serving; it is not starting. Stop it, then start the TUI again. {detail}"
                    ));
                }
                let due = last_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER);
                if due && waited >= WAIT_NOTICE_AFTER {
                    eprintln!(
                        "future-tui: waiting for the Agent another process is starting \
                         ({waited:?} of {WAIT_FOR_OWNER:?}). {detail}"
                    );
                    last_notice = Some(Instant::now());
                }
                tokio::time::sleep(WAIT_PROBE_INTERVAL).await;
            }
            LocalAgentState::Blocked(reason) => return Err(reason),
            // Attached already, or free to start: the ordinary launch decides.
            LocalAgentState::Attached | LocalAgentState::Startable => {
                return ensure_agent_running(configured).await;
            }
        }
    }
}

/// Keep watching the Agent for as long as the TUI runs.
///
/// The TUI's first launch happens once, which is why an Agent that dies later
/// (killed, OOM, its owning Desktop quitting) leaves the terminal reconnecting
/// forever. This task closes that: it notices the absence, starts a replacement
/// when nobody holds the instance lock, waits for the process that does, and rate
/// limits its own restarts so an Agent the host keeps killing cannot become a
/// loop. `initial` is the guard the launch produced, so an owned sidecar keeps
/// living exactly as long as it did before.
///
/// The entry point calls this from outside async context, where a bare
/// `tokio::spawn` has no reactor and panics, so the caller hands over the
/// runtime's handle instead. The task lives exactly as long as that runtime.
pub fn spawn_agent_supervision(
    handle: &tokio::runtime::Handle,
    configured: String,
    initial: Option<OwnedAgent>,
) {
    handle.spawn(async move {
        let mut guard = initial;
        let mut budget = RestartBudget::new();
        let mut waiting_since: Option<Instant> = None;
        let mut lost_notice: Option<Instant> = None;
        let mut reachable_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(PROBE_INTERVAL).await;
            match probe_local_agent(&configured, PROBE_CONNECT, PROBE_HANDSHAKE).await {
                LocalAgentState::Attached => {
                    if waiting_since.take().is_some() {
                        eprintln!("future-tui: the Agent is serving again.");
                    }
                    lost_notice = None;
                    // Only a lasting recovery is a success: an Agent the host
                    // keeps killing becomes ready on every lap, and clearing the
                    // budget on "attached" would reset the breaker each time.
                    let now = Instant::now();
                    let since = *reachable_since.get_or_insert(now);
                    if Attached::of(since, now) == Attached::Stable {
                        budget.reset();
                    }
                }
                LocalAgentState::Busy(detail) => {
                    let now = Instant::now();
                    let since = *waiting_since.get_or_insert(now);
                    let waited = now.duration_since(since);
                    // Say it once, then every minute; say nothing at all once
                    // the wait is over.
                    let due = lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER)
                        && waited >= WAIT_NOTICE_AFTER;
                    if due {
                        eprintln!(
                            "future-tui: waiting for the Agent another process is starting \
                             ({waited:?} of {WAIT_FOR_OWNER:?}). {detail}"
                        );
                        lost_notice = Some(now);
                    }
                    if waited >= WAIT_FOR_OWNER {
                        eprintln!(
                            "future-tui: no Agent has served this data root for {WAIT_FOR_OWNER:?}; \
                             still waiting. The TUI keeps reconnecting, and a new attempt starts \
                             when the lock is released."
                        );
                        waiting_since = None;
                        lost_notice = Some(now);
                    }
                }
                LocalAgentState::Blocked(reason) => {
                    // Not repairable by waiting (version mismatch, explicit
                    // endpoint). Keep the connection banner and look again later.
                    if lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER) {
                        eprintln!("future-tui: cannot use the Agent: {reason}");
                        lost_notice = Some(Instant::now());
                    }
                }
                LocalAgentState::Startable => {
                    waiting_since = None;
                    reachable_since = None;
                    let now = Instant::now();
                    if !budget.allow(now) {
                        if lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER) {
                            eprintln!(
                                "future-tui: the Agent keeps exiting right after it starts; \
                                 not restarting it for {:?}. Check the host log: {}",
                                budget.open_for(now).unwrap_or_default(),
                                configured
                            );
                            lost_notice = Some(now);
                        }
                        continue;
                    }
                    // Nothing answers: a guard we still hold is dead weight (and
                    // possibly a zombie child that would keep the endpoint).
                    drop(guard.take());
                    match ensure_agent_running(&configured).await {
                        Ok(new_guard) => {
                            guard = new_guard;
                            eprintln!("future-tui: the Agent was gone; started a new one.");
                            reachable_since = None;
                            lost_notice = None;
                        }
                        Err(error) => {
                            if lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER) {
                                eprintln!("future-tui: could not start an Agent: {error}");
                                lost_notice = Some(Instant::now());
                            }
                            tokio::time::sleep(restart_delay(2)).await;
                        }
                    }
                }
            }
        }
    });
}

/// The launch's final word: nothing answered the probe and the one sidecar
/// attempt failed for the stated reason.
fn sidecar_launch_failed(failure: String) -> String {
    format!("no Future Agent was reachable and the sidecar could not be started ({failure})")
}

/// How long the failure report waits for each drain thread to reach EOF once
/// the sidecar is dead. EOF is normally immediate (the child is gone and
/// reaped), so the bound only keeps a wedged thread from delaying the report —
/// it would then keep writing into buffers nobody reads again, held alive by
/// its own `Arc`s until the thread ends.
const DRAIN_QUIESCE: Duration = Duration::from_millis(250);

/// Bytes kept of the sidecar's output per stream. Fixed on purpose: the drain
/// threads empty the pipes continuously so the child is never blocked by them,
/// and a failure report must stay a report however chatty a dying sidecar is.
const TAIL_CAPACITY: usize = 4 * 1024;

/// Marker prefixed to a snapshot whose head was dropped, so the report never
/// claims completeness it does not have.
const TAIL_TRUNCATED_MARK: &str = "[…truncated] ";

/// Read size of one drain turn; the bound the report cares about is
/// [`TAIL_CAPACITY`], enforced by the shared buffer, not by the reader.
const DRAIN_CHUNK: usize = 8 * 1024;

/// The last [`TAIL_CAPACITY`] bytes of one output stream, kept so a failed
/// launch can quote the sidecar's own failure reason. Once full, the oldest
/// bytes are dropped and the loss is marked rather than hidden.
struct TailBuffer {
    bytes: VecDeque<u8>,
    truncated: bool,
}

impl TailBuffer {
    fn new() -> Self {
        Self {
            bytes: VecDeque::with_capacity(TAIL_CAPACITY),
            truncated: false,
        }
    }

    fn push_bytes(&mut self, new_bytes: &[u8]) {
        if new_bytes.len() >= TAIL_CAPACITY {
            // A single write larger than the whole window: everything before
            // its own tail is gone, including whatever was buffered.
            self.truncated = true;
            self.bytes.clear();
            self.bytes
                .extend(new_bytes[new_bytes.len() - TAIL_CAPACITY..].iter().copied());
            return;
        }
        let overflow = (self.bytes.len() + new_bytes.len()).saturating_sub(TAIL_CAPACITY);
        if overflow > 0 {
            self.truncated = true;
            self.bytes.drain(..overflow);
        }
        self.bytes.extend(new_bytes.iter().copied());
    }

    fn snapshot(&self) -> String {
        let bytes: Vec<u8> = self.bytes.iter().copied().collect();
        // Output is nominally UTF-8 (tracing logs, anyhow chains), but a
        // multi-byte character split at the window edge must not lose the
        // report: replacement is better than failure here.
        let content = String::from_utf8_lossy(&bytes);
        if self.truncated {
            format!("{TAIL_TRUNCATED_MARK}{content}")
        } else {
            content.into_owned()
        }
    }
}

/// The two drain threads and the failure report meet only here: one tail per
/// stream, shared for as long as either side holds its `Arc`.
type SharedTail = Arc<Mutex<TailBuffer>>;

/// Read `pipe` to EOF into `tail`, then signal `done`. Runs on its own
/// detached thread — the only cleanup is the signal, whose send failure just
/// means the failure report already gave up waiting.
fn drain_pipe<R: Read>(mut pipe: R, tail: SharedTail, done: mpsc::Sender<()>) {
    let mut chunk = [0u8; DRAIN_CHUNK];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if let Ok(mut tail) = tail.lock() {
                    tail.push_bytes(&chunk[..read]);
                }
            }
        }
    }
    let _ = done.send(());
}

/// Hand the sidecar's piped stdout and stderr to one drain thread each.
///
/// The pipes are taken out of the `Child` before the threads start: from here
/// on they belong to the drains, so the success path can move the `Child` into
/// an [`OwnedAgent`] without dragging file handles along. The receivers
/// complete the handshake in [`tail_after_exit`]; on the success path they are
/// dropped unread, and the threads end on EOF once the guard kills the child.
fn spawn_output_drains(
    child: &mut Child,
) -> (
    SharedTail,
    SharedTail,
    mpsc::Receiver<()>,
    mpsc::Receiver<()>,
) {
    let stdout_tail: SharedTail = Arc::new(Mutex::new(TailBuffer::new()));
    let stderr_tail: SharedTail = Arc::new(Mutex::new(TailBuffer::new()));
    let (stdout_done, stdout_done_rx) = mpsc::channel::<()>();
    let (stderr_done, stderr_done_rx) = mpsc::channel::<()>();
    if let Some(pipe) = child.stdout.take() {
        let tail = Arc::clone(&stdout_tail);
        std::thread::spawn(move || drain_pipe(pipe, tail, stdout_done));
    }
    if let Some(pipe) = child.stderr.take() {
        let tail = Arc::clone(&stderr_tail);
        std::thread::spawn(move || drain_pipe(pipe, tail, stderr_done));
    }
    (stdout_tail, stderr_tail, stdout_done_rx, stderr_done_rx)
}

/// The sidecar's own words for the failure report, taken once it is dead: wait
/// a bounded moment for each drain to reach EOF, then snapshot both tails.
/// Empty output yields an empty string, so a silent sidecar stays unquoted.
fn tail_after_exit(
    stdout_done: &mpsc::Receiver<()>,
    stderr_done: &mpsc::Receiver<()>,
    stdout_tail: &SharedTail,
    stderr_tail: &SharedTail,
) -> String {
    let _ = stdout_done.recv_timeout(DRAIN_QUIESCE);
    let _ = stderr_done.recv_timeout(DRAIN_QUIESCE);
    let stdout = stdout_tail
        .lock()
        .map(|tail| tail.snapshot())
        .unwrap_or_default();
    let stderr = stderr_tail
        .lock()
        .map(|tail| tail.snapshot())
        .unwrap_or_default();
    // Trailing newlines are line terminators, not content; embedded ones stay
    // so a multi-line error from the agent is shown as the agent wrote it.
    let stdout = stdout.trim_end();
    let stderr = stderr.trim_end();
    match (!stdout.is_empty(), !stderr.is_empty()) {
        (true, true) => format!("{stdout}\n{stderr}"),
        (true, false) => stdout.to_string(),
        (false, true) => stderr.to_string(),
        (false, false) => String::new(),
    }
}

/// Append the captured output to a failure reason. One framing line, but any
/// newlines inside the capture are kept intact — the agent's error chain is
/// the user's best clue, not something to flatten.
fn attach_last_output(failure: &mut String, tail: String) {
    if !tail.is_empty() {
        failure.push_str("; last output: ");
        failure.push_str(&tail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sidecar command is pinned to the unified binary's `agent` subcommand
    /// with endpoint auto-discovery: a caller-chosen address would launch an
    /// agent the TUI cannot find again on the instance endpoint.
    #[test]
    fn sidecar_args_are_the_agent_subcommand_in_auto_mode() {
        assert_eq!(
            sidecar_args(),
            vec![
                "agent".to_string(),
                "--grpc-addr".to_string(),
                "auto".to_string()
            ]
        );
    }

    /// Only the unified `future` binary may be re-executed as the sidecar: the
    /// standalone `future-tui` would recurse into a second TUI instead of an
    /// agent, and the agent's own `future-agent` binary takes no `agent`
    /// subcommand either.
    #[test]
    fn only_the_unified_future_binary_may_spawn_the_sidecar() {
        assert!(check_sidecar_program(Path::new("/opt/future/bin/future")).is_ok());
        assert!(check_sidecar_program(Path::new("/usr/local/bin/future-agent")).is_err());

        let error = check_sidecar_program(Path::new("/opt/future/bin/future-tui"))
            .expect_err("the standalone TUI must not be respawned as its own sidecar");
        assert!(
            error.contains("cannot start an agent sidecar") && error.contains("future tui"),
            "{error}"
        );
    }

    /// A long-running child that exits on its own only long after the test.
    fn spawn_sleeper() -> Child {
        // Pinned to an absolute path (see `test_env::absolute_tool`): this test
        // does not take the env lock, and a peer test's `PATH` swap must not decide
        // whether this child starts.
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new(crate::test_env::absolute_tool("cmd"));
            command.args(["/c", "ping -n 60 127.0.0.1 > nul"]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new(crate::test_env::absolute_tool("sleep"));
            command.arg("60");
            command
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a sleeper")
    }

    /// Whether `pid` still exists, asked of the OS rather than of our own
    /// handle (a `Child` we still hold would answer "running" about a corpse).
    fn process_is_running(pid: u32) -> bool {
        #[cfg(windows)]
        {
            let out = Command::new(crate::test_env::absolute_tool("tasklist"))
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .expect("tasklist");
            String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
        }
        #[cfg(unix)]
        {
            Command::new(crate::test_env::absolute_tool("ps"))
                .args(["-p", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("ps")
                .success()
        }
    }

    /// The TUI owns this guard: dropping it must kill and reap the sidecar it
    /// started, or every TUI exit would leak a `future agent` process.
    ///
    /// The liveness check runs in a *separate* process and is first shown to
    /// answer "running" for the live child — so "gone" afterwards cannot be the
    /// observer simply never seeing anything.
    #[test]
    fn dropping_the_owned_agent_kills_its_child() {
        let child = spawn_sleeper();
        let pid = child.id();
        assert!(
            process_is_running(pid),
            "the observer must be able to see a live process"
        );

        drop(OwnedAgent { child });

        assert!(
            !process_is_running(pid),
            "a dropped OwnedAgent must kill the child it owns (pid {pid})"
        );
    }

    /// The `cfg!(test)` guard: a unit test must never launch a real sidecar, so
    /// the probe's failure must surface as this refusal and not as a spawn.
    #[tokio::test]
    async fn ensure_agent_running_refuses_an_unavailable_explicit_endpoint() {
        // Port 1 is never a running agent, so the health probe fails and the
        // launch path is entered.
        let error = match ensure_agent_running("127.0.0.1:1").await {
            Ok(_) => panic!("a test build must not start an agent"),
            Err(error) => error,
        };
        assert!(error.contains("no local fallback"), "{error}");
    }

    /// Segmented writes within the capacity are preserved verbatim: the tail
    /// is a buffer, not a rewriter, and nothing may be marked as dropped.
    #[test]
    fn tail_buffer_preserves_output_within_capacity() {
        let mut tail = TailBuffer::new();
        tail.push_bytes(b"starting\n");
        tail.push_bytes(b"boom: bad config\n");
        assert_eq!(tail.snapshot(), "starting\nboom: bad config\n");
        assert!(!tail.truncated, "nothing was dropped, so nothing is marked");
    }

    /// Output beyond the capacity drops the *head* — the oldest bytes — and
    /// marks the snapshot, and what is kept is exactly the last window.
    #[test]
    fn tail_buffer_drops_its_head_beyond_capacity() {
        let mut tail = TailBuffer::new();
        let whole = vec![b'a'; TAIL_CAPACITY + 32];
        tail.push_bytes(&whole);

        let snapshot = tail.snapshot();
        assert!(snapshot.starts_with(TAIL_TRUNCATED_MARK), "{snapshot:?}");
        let content = snapshot
            .strip_prefix(TAIL_TRUNCATED_MARK)
            .expect("the truncation mark");
        assert!(
            content.len() <= TAIL_CAPACITY,
            "the snapshot must never exceed the capacity: {} bytes",
            content.len()
        );
        assert_eq!(content.len(), TAIL_CAPACITY, "the full window is kept");
        assert!(content.bytes().all(|byte| byte == b'a'), "{snapshot:?}");
    }

    /// The capacity is a window across writes, not a per-write limit: a full
    /// window followed by a small write keeps the small write and the big
    /// write's own tail.
    #[test]
    fn tail_buffer_keeps_the_tail_across_writes() {
        let mut tail = TailBuffer::new();
        tail.push_bytes(&vec![b'a'; TAIL_CAPACITY]);
        tail.push_bytes(b"final line\n");

        let snapshot = tail.snapshot();
        assert!(snapshot.starts_with(TAIL_TRUNCATED_MARK), "{snapshot:?}");
        let content = snapshot
            .strip_prefix(TAIL_TRUNCATED_MARK)
            .expect("the truncation mark");
        assert!(
            content.ends_with("final line\n"),
            "the newest write must survive: {snapshot:?}"
        );
        assert_eq!(content.len(), TAIL_CAPACITY);
    }
}
