//! Project-local on-disk projection.
//!
//! The event ledger stays the source of truth; this layer MATERIALIZES the
//! active-state markdown the agent reads, plus the goal document:
//!
//!   <project>/GOAL.md                          — goal document (optional)
//!   <cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md — active state
//!     (todo anchors `<!-- future-loop:todo ... -->`)
//!
//! Everything lives inside the project (no external runtime root, no
//! reference-control-plane layout mirrors).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::json;

use crate::state::{Goal, TaskClass, Todo, TodoStatus};

/// reference URL-encodes spaces (%20) in anchor values.
fn url_encode(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('+', "%2B")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
        .replace('\t', "%09")
        .replace('<', "%3C")
        .replace('>', "%3E")
}

/// RFC3339-ish timestamp matching reference (e.g. 2026-08-05T11:03:14+08:00).
pub fn rfc3339(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    let dt = Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .unwrap_or_else(Local::now);
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// Map our TaskClass to the reference task_class value (already equal via serde,
/// but keep the mapping explicit here).
pub fn future_loop_task_class(c: TaskClass) -> &'static str {
    match c {
        TaskClass::Advancement => "advancement_task",
        TaskClass::UserGate => "user_gate",
        TaskClass::UserAction => "user_action",
        TaskClass::Monitor => "continuous_monitor",
        TaskClass::Blocker => "blocker",
        TaskClass::Coordination => "coordination",
    }
}

pub fn future_loop_status(s: TodoStatus) -> &'static str {
    match s {
        TodoStatus::Open => "open",
        TodoStatus::Done => "done",
        TodoStatus::Superseded => "superseded",
        TodoStatus::Deferred => "deferred",
        TodoStatus::Blocked => "blocked",
    }
}

// ── GOAL.md ────────────────────────────────────────────────────────────────

pub fn write_goal_doc(project: &str, objective: &str) -> Result<()> {
    fs::write(Path::new(project).join("GOAL.md"), format!("{objective}\n")).context("write GOAL.md")
}

// ── <state>/ACTIVE_GOAL_STATE.md (active-state markdown) ───────────────────

/// Write the active-state projection into the goal's state directory
/// (`<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md`).
pub fn write_active_state(goal_dir: &Path, goal: &Goal) -> Result<()> {
    fs::create_dir_all(goal_dir)?;
    let lock = acquire_active_state_lock(goal_dir)?;
    let md = render_active_state(goal);
    let result =
        fs::write(goal_dir.join("ACTIVE_GOAL_STATE.md"), md).context("write ACTIVE_GOAL_STATE.md");
    release_active_state_lock(&lock);
    result
}

// ── ACTIVE_GOAL_STATE.md.lock (liveness-checked sidecar lock) ─────────────
//
// The active-state markdown has a LoopX-compatible sidecar lock. Acquiring
// writes OUR pid into the lock file; on conflict we read the holder's pid and
// probe liveness (`kill -0`): a live holder is waited for up to
// [`LIVE_HOLDER_WAIT`] (it holds the lock only for one projection write) and
// then reported as a hard error, while a dead holder is cleared and taken over
// at once. An EMPTY lock is waited for [`EMPTY_LOCK_WAIT`] and then taken over
// too — see that constant for why a long wait there is wrong rather than safe.

/// The sidecar lock inside a goal's state directory.
const ACTIVE_STATE_LOCK: &str = "ACTIVE_GOAL_STATE.md.lock";

/// How long an EMPTY (no pid) lock is left alone before it is taken over.
///
/// `create_new` publishes the file and the pid is written immediately after, so
/// an empty lock means one of exactly two things: another process sits between
/// those two syscalls (microseconds), or a process died there. A short wait
/// separates them; a long one does not, and it is not free. This used to be a
/// ten-minute refusal, during which every command that refreshes the projection
/// failed — while its ledger change had already been committed — and reported
/// the operation as failed.
///
/// Taking over a live-but-slow writer is harmless by construction: the file this
/// lock guards is a regenerable projection of the ledger, so the worst case is
/// that it gets written twice with the same content. The ledger's own advisory
/// lock (see the store) is the real concurrency guard.
const EMPTY_LOCK_WAIT: Duration = Duration::from_millis(250);

/// How long to wait for a live holder to release the lock before reporting it
/// as held. The lock covers one projection write, so this only ever waits for
/// a concurrent writer to finish.
const LIVE_HOLDER_WAIT: Duration = Duration::from_millis(500);

/// Poll interval while waiting on a holder or an empty lock.
const LIVE_HOLDER_POLL: Duration = Duration::from_millis(5);

/// Bound on the acquire loop, so a pathological remove/create interleaving
/// cannot spin forever. Reaching it means heavy contention, not a stuck lock.
const MAX_LOCK_ATTEMPTS: usize = 512;

/// Windows `ERROR_ACCESS_DENIED`, reported by every operation on a file another
/// writer is concurrently unlinking (see [`delete_pending_retry`]).
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;

/// Retries allowed for the Windows delete-pending window. The window closes as
/// soon as the unlinking writer drops its handle (microseconds), so a handful
/// of polls is generous; exhausting the budget means the error really is a
/// permission problem and is surfaced.
#[cfg(windows)]
const DELETE_PENDING_RETRIES: usize = 64;

/// Is this `create_new` failure just another writer's transient state rather
/// than a real error?
///
/// On Unix the only contention signal is `AlreadyExists`. Windows has a second
/// one: between `remove_file` and the kernel finishing the unlink the name is
/// *delete-pending*, and `create_new` on it reports `ERROR_ACCESS_DENIED` — not
/// `AlreadyExists`, not `NotFound`. Four workers claiming at once hit that
/// window routinely, so it must fall through to the classify/remove path below
/// (which sorts out whether the lock is really still there) instead of being
/// reported as a failure. Anything else — a genuinely unwritable directory,
/// say — still surfaces as an error.
fn create_is_contention(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return true;
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(ERROR_ACCESS_DENIED)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Windows quirk, removal side: `remove_file` on a delete-pending name also
/// reports `ERROR_ACCESS_DENIED`, though the unlink the caller wanted is already
/// underway. Sleep and let the caller retry, up to a bounded budget; any other
/// error, or an exhausted budget, is reported unchanged — a real permission
/// problem must still name itself.
#[cfg(windows)]
fn delete_pending_retry(retries: &mut usize, error: &std::io::Error) -> bool {
    if error.raw_os_error() != Some(ERROR_ACCESS_DENIED) || *retries >= DELETE_PENDING_RETRIES {
        return false;
    }
    *retries += 1;
    std::thread::sleep(LIVE_HOLDER_POLL);
    true
}

/// Non-Windows has no delete-pending state: unlink succeeds even while other
/// handles are open, so a removal error is always real.
#[cfg(not(windows))]
fn delete_pending_retry(_retries: &mut usize, _error: &std::io::Error) -> bool {
    false
}

/// What a lock file says about its holder.
enum LockState {
    /// Gone — released between our create attempt and this read.
    Released,
    /// Held by a process that is still alive.
    Live(u32),
    /// Held by a pid that no longer exists.
    Dead,
    /// Empty or unreadable: a writer mid-acquire, or one that died there.
    Empty,
}

/// Acquire the `ACTIVE_GOAL_STATE.md.lock` sidecar for `goal_dir`, writing this
/// process's pid into the file. On success the caller owns the lock and MUST
/// release it with [`release_active_state_lock`]; on failure — a live holder
/// that outlasted [`LIVE_HOLDER_WAIT`], or sustained contention — the error says
/// which.
pub fn acquire_active_state_lock(goal_dir: &Path) -> Result<PathBuf> {
    acquire_active_state_lock_with(goal_dir, EMPTY_LOCK_WAIT)
}

/// Testable variant: `empty_wait` overrides [`EMPTY_LOCK_WAIT`].
fn acquire_active_state_lock_with(goal_dir: &Path, empty_wait: Duration) -> Result<PathBuf> {
    fs::create_dir_all(goal_dir)?;
    let lock_path = goal_dir.join(ACTIVE_STATE_LOCK);
    // Both deadlines start when the state is first observed, not when the call
    // begins, so a wait for an earlier holder does not eat the later one's budget.
    let mut live_deadline: Option<std::time::Instant> = None;
    let mut empty_deadline: Option<std::time::Instant> = None;
    let mut delete_pending_retries = 0usize;
    for _ in 0..MAX_LOCK_ATTEMPTS {
        // Publishing the pid is what makes us the holder; `create_new` is the
        // atomic part. The file is briefly visible without a pid — a reader has
        // to tolerate that instead of waiting for it to age out.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                writeln!(file, "{}", std::process::id()).context("write pid into lock")?;
                return Ok(lock_path);
            }
            Err(error) if create_is_contention(&error) => {}
            Err(error) => return Err(error).context(format!("create {ACTIVE_STATE_LOCK}")),
        }
        match classify_lock(&lock_path) {
            LockState::Released => continue,
            LockState::Live(pid) => {
                let deadline = *live_deadline
                    .get_or_insert_with(|| std::time::Instant::now() + LIVE_HOLDER_WAIT);
                if std::time::Instant::now() >= deadline {
                    bail!("{ACTIVE_STATE_LOCK} held by pid {pid}");
                }
                std::thread::sleep(LIVE_HOLDER_POLL);
            }
            LockState::Dead => {
                if let Err(error) = remove_lock_file(&lock_path) {
                    if delete_pending_retry(&mut delete_pending_retries, &error) {
                        continue;
                    }
                    return Err(error).context("remove dead-holder lock");
                }
            }
            LockState::Empty => {
                let deadline =
                    *empty_deadline.get_or_insert_with(|| std::time::Instant::now() + empty_wait);
                if std::time::Instant::now() >= deadline {
                    // Still no pid after the grace period: the writer that created
                    // it is gone, not slow.
                    if let Err(error) = remove_lock_file(&lock_path) {
                        if delete_pending_retry(&mut delete_pending_retries, &error) {
                            continue;
                        }
                        return Err(error).context("remove abandoned lock");
                    }
                } else {
                    std::thread::sleep(LIVE_HOLDER_POLL);
                }
            }
        }
    }
    bail!("could not acquire {ACTIVE_STATE_LOCK} (contended)")
}

/// Read the lock file and say what it means. Classification is a pure read; the
/// waiting happens in the acquire loop.
fn classify_lock(lock_path: &Path) -> LockState {
    let raw = match fs::read_to_string(lock_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return LockState::Released,
        // Unreadable for any other reason: treat like empty, so a permissions
        // oddity cannot wedge the projection forever.
        Err(_) => String::new(),
    };
    match raw.trim().parse::<u32>() {
        Ok(pid) if pid_alive(pid) => LockState::Live(pid),
        Ok(_) => LockState::Dead,
        Err(_) => LockState::Empty,
    }
}

/// Remove a lock file we decided to take over; a concurrent cleanup is fine.
fn remove_lock_file(lock_path: &Path) -> std::io::Result<()> {
    match fs::remove_file(lock_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Release the lock sidecar acquired by [`acquire_active_state_lock`].
/// Best-effort: the lock is a liveness signal, not the store's real
/// concurrency guard (that remains the advisory file lock in the store).
pub fn release_active_state_lock(lock_path: &Path) {
    let _ = fs::remove_file(lock_path);
}

/// The user's home directory as a string (`""` when nothing resolves).
///
/// `$HOME` wins (POSIX, and a redirected/portable home), then `USERPROFILE` —
/// Windows shells set no `HOME`, so a `$HOME`-only lookup silently disabled
/// `~` shortening and absolute-home leak detection there. The platform profile
/// API is not consulted: it observes neither variable, which would ignore a
/// redirected home (and the isolated homes the tests install).
pub fn home_dir() -> String {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .find(|path| path.is_absolute() && !path.as_os_str().is_empty())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Is the process with this pid still alive? Unix probes with `kill -0`
/// (no signal delivered); Windows opens the process and reads its exit code.
/// A process we may not query at all counts as alive — a lock is never stolen
/// from a possibly live holder (empty-lock aging still applies).
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 performs existence checking only; no signal is sent.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    // EPERM: the process exists but we may not signal it → alive.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Windows probe: a live process is one we can open and whose exit code is
/// still `STILL_ACTIVE`. A pid whose process is gone (or was never there)
/// fails `OpenProcess` with `ERROR_INVALID_PARAMETER`; `ERROR_ACCESS_DENIED`
/// means it exists but is protected, which counts as alive.
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess returns an owned handle that is closed below; the
    // exit-code buffer is a live local and every failure path is handled.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut exit_code = 0u32;
        let alive =
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32;
        CloseHandle(handle);
        alive
    }
}

/// Platforms with neither a signal probe nor a Win32 process handle: report
/// alive so a lock is never stolen from a possibly live holder.
#[cfg(not(any(unix, windows)))]
pub fn pid_alive(_pid: u32) -> bool {
    true
}

// ── <state>/runs/ (per-run history + index) ────────────────────────────────

/// Append one run's files (JSON + markdown + index row) into the goal's
/// state directory (`<cwd>/.future/loop/goals/<id>/runs/`).
pub fn write_run(goal_dir: &Path, goal_id: &str, record: &crate::state::RunRecord) -> Result<()> {
    let dir = goal_dir.join("runs");
    fs::create_dir_all(&dir)?;
    // Run-file names: 2026-08-05T11-03-14-08-00 (offset without +/:)
    let now = chrono::Local::now();
    // "+0800" -> "08-00" (dash between hours and minutes).
    let z = now.format("%z").to_string();
    let digits = z.trim_start_matches(['+', '-']);
    let sign = if z.starts_with('-') { "-" } else { "" };
    let (hh, mm) = digits.split_at(2);
    let offset = format!("{sign}{hh}-{mm}");
    let ts = format!(
        "{}-{offset}-{}",
        now.format("%Y-%m-%dT%H-%M-%S"),
        uuid::Uuid::new_v4().simple()
    );

    let json_payload = json!({
        "goal_id": goal_id,
        "timestamp": rfc3339(record.recorded_at),
        "turn": record.turn,
        "todo_id": record.todo_id,
        "run_id": record.run_id,
        "agent_id": record.agent_id,
        "terminal_state": record.terminal_state,
        "tools": record.tools,
        "tokens_in": record.tokens_in_delta,
        "tokens_out": record.tokens_out_delta,
        "cost": record.cost_delta,
        "evidence": record.evidence,
        "error": record.error,
    });
    let json_path = dir.join(format!("{ts}.json"));
    fs::write(json_path, serde_json::to_string_pretty(&json_payload)?)?;

    let mut md = String::new();
    md.push_str(&format!("# Run {ts}\n\n"));
    md.push_str(&format!("- goal_id: {goal_id}\n"));
    md.push_str(&format!("- turn: {}\n", record.turn));
    md.push_str(&format!("- todo: {}\n", record.todo_id));
    md.push_str(&format!("- state: {}\n", record.terminal_state));
    md.push_str(&format!("- tools: {}\n", record.tools.join(", ")));
    md.push_str(&format!("- evidence: {}\n", record.evidence));
    fs::write(dir.join(format!("{ts}.md")), md)?;

    // NOTE: the run index is no longer appended here. It was a per-process
    // `OpenOptions::append` with no cross-process lock, so N concurrent
    // workers writing a run at once could interleave index rows and drift the
    // read model (the `run_index drifted — rebuilt` self-heal noise). The
    // index is now a pure projection derived on read from the run files in
    // this directory (see `run_index::load_run_index`) — the run JSON/MD
    // files above remain the single source of truth and are never contended
    // (each is a unique file).
    Ok(())
}

/// Render the ACTIVE_GOAL_STATE.md markdown (pub for the G-4 multi-projection
/// layer, which grades and redacts the same render through privacy lenses).
pub fn render_active_state(goal: &Goal) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("status: active\n");
    out.push_str("owner_mode: goal\n");
    out.push_str(&format!("objective: {:?}\n", goal.objective));
    out.push_str(&format!(
        "updated_at: {}\n",
        rfc3339(crate::state::now_epoch())
    ));
    out.push_str(&format!("adapter_id: {}\n", goal.goal_id));
    out.push_str("---\n\n");
    out.push_str("# Active Goal State\n\n");
    out.push_str("## Objective\n\n");
    out.push_str(&format!("{}\n\n", goal.objective));
    out.push_str("## Authority Sources\n\n");
    if std::path::Path::new(&goal.cwd).join("GOAL.md").exists() {
        out.push_str("- Primary goal document: `GOAL.md`\n\n");
    } else {
        out.push_str("- No explicit goal document was provided during bootstrap.\n\n");
    }
    out.push_str("## Operating Contract\n\n");
    for line in [
        "Treat this file as the durable goal state for future agent ticks.",
        "Treat the authority sources above as the first context to inspect before acting.",
        "Read current project evidence before choosing the next action.",
        "Run a bounded progress segment when useful; it does not have to be one tiny step.",
        "Keep private evidence, credentials, local paths, and raw logs out of public commits.",
        "End each tick with changed files, validation, residual risk, and the next action.",
    ] {
        out.push_str(&format!("- {line}\n"));
    }
    out.push('\n');
    out.push_str("## Execution Profile\n\n");
    out.push_str(&format!(
        "- `cadence={} minimum=multi_surface_or_implementation include=coherent_artifact,targeted_validation,state_writeback spend_rule={} small_streak_threshold=2`\n",
        goal.execution_profile.cadence, goal.execution_profile.spend_rule
    ));
    out.push_str("- Repeated small-scale follow-through should expand the next delivery batch or report a blocker before spending quota.\n\n");
    out.push_str("## Non-Goals\n\n");
    out.push_str(
        "- Do not perform irreversible production operations without explicit approval.\n",
    );
    out.push_str("- Do not publish private project evidence.\n");
    out.push_str(
        "- Do not optimize for activity if no useful artifact or decision can be produced.\n\n",
    );

    // Todos, split by role (reference sections).
    let user_todos: Vec<&Todo> = goal
        .todos
        .iter()
        .filter(|t| t.role == crate::state::TodoRole::User)
        .collect();
    let agent_todos: Vec<&Todo> = goal
        .todos
        .iter()
        .filter(|t| t.role == crate::state::TodoRole::Agent)
        .collect();
    out.push_str("## User Todo / Owner Review Reading Queue\n\n");
    for t in &user_todos {
        out.push_str(&todo_line(t, &goal.history));
    }
    out.push('\n');
    out.push_str("## Agent Todo\n\n");
    for t in &agent_todos {
        out.push_str(&todo_line(t, &goal.history));
    }
    out.push('\n');
    out.push_str("## Next Action\n\n");
    let next_default = "Initial routing is owned by the connected domain adapter.";
    out.push_str(&format!(
        "- {}\n\n",
        goal.next_action.as_deref().unwrap_or(next_default)
    ));
    out.push_str("## Recent User Feedback\n\n");
    out.push_str("- Initialized by `future-loop bootstrap`.\n\n");
    out.push_str("## Progress Ledger\n\n");
    if goal.history.is_empty() {
        out.push_str("- Created the initial goal state and registry connection.\n");
    } else {
        for r in goal.history.iter().rev().take(5) {
            out.push_str(&format!(
                "- turn {} ({}): todo={} state={} tools=[{}] evidence={}\n",
                r.turn,
                rfc3339(r.recorded_at),
                r.todo_id,
                r.terminal_state,
                r.tools.join(","),
                crate::decision::truncate(&r.evidence, 120)
            ));
        }
    }
    out
}

/// One todo bullet with the reference `<!-- future-loop:todo ... -->` anchor.
fn todo_line(t: &Todo, _history: &[crate::state::RunRecord]) -> String {
    let is_default_advancement = t.class == TaskClass::Advancement && t.action_kind.is_none();
    // LoopX: deferred todos render with a "-" checkbox; done with "x".
    let checkbox = if t.status == TodoStatus::Deferred {
        "-"
    } else if t.status == TodoStatus::Done {
        "x"
    } else {
        " "
    };
    let mut line = if is_default_advancement {
        format!(
            "- [{checkbox}] {}\n  <!-- future-loop:todo todo_id={} status={}",
            t.text,
            t.id,
            future_loop_status(t.status),
        )
    } else {
        format!(
            "- [{checkbox}] {}\n  <!-- future-loop:todo todo_id={} status={} task_class={}",
            t.text,
            t.id,
            future_loop_status(t.status),
            future_loop_task_class(t.class),
        )
    };
    if let Some(ak) = &t.action_kind {
        line.push_str(&format!(" action_kind={ak}"));
    } else if t.class == TaskClass::UserGate {
        line.push_str(" action_kind=goal_decision");
    }
    if let Some(rw) = &t.resume_when_text {
        line.push_str(&format!(" resume_when={}", url_encode(rw)));
    }
    // G-12: monitor metadata in the anchor (target / policy / cadence).
    if let Some(target) = &t.monitor_target {
        line.push_str(&format!(" monitor_target={}", url_encode(target)));
    }
    if let Some(policy) = &t.monitor_policy {
        line.push_str(&format!(" monitor_policy={}", url_encode(policy)));
    }
    if let Some(cadence) = &t.monitor_cadence {
        line.push_str(&format!(" cadence={cadence}"));
    }
    if let Some(note) = &t.note {
        line.push_str(&format!(" note={}", url_encode(note)));
    }
    if t.goal_bound {
        line.push_str(" goal_bound=true");
    }
    if t.global_gate {
        line.push_str(" global_gate=true");
    }
    if let Some(owner) = &t.claimed_by {
        line.push_str(&format!(" claimed_by={owner}"));
    }
    if t.no_follow_up {
        line.push_str(" no_followup=true");
    }
    if t.status == TodoStatus::Done {
        // reference completed anchors carry URL-encoded evidence + completed_at.
        if let Some(ev) = t.evidence.as_deref().filter(|e| !e.is_empty()) {
            line.push_str(&format!(" evidence={}", url_encode(ev)));
        }
        if let Some(ts) = t.completed_at {
            line.push_str(&format!(
                " completed_at={}",
                rfc3339(ts).replace('+', "%2B")
            ));
        }
    }
    line.push_str(" updated_at=");
    // reference encodes only '+' -> %2B; colons stay literal.
    let ts = rfc3339(t.updated_at).replace('+', "%2B");
    line.push_str(&ts);
    line.push_str(" -->\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_path(dir: &Path) -> PathBuf {
        dir.join("ACTIVE_GOAL_STATE.md.lock")
    }

    #[test]
    fn live_holder_pid_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Our own pid is certainly alive → the lock must be reported held.
        let own_pid = std::process::id();
        std::fs::write(lock_path(dir.path()), format!("{own_pid}\n")).unwrap();
        let err = acquire_active_state_lock(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("held by pid"), "unexpected error: {msg}");
        assert!(
            msg.contains(&own_pid.to_string()),
            "unexpected error: {msg}"
        );
        // The live lock is left untouched.
        assert!(lock_path(dir.path()).exists());
    }

    /// Spawn a child and reap it — its pid is now guaranteed dead.
    fn reaped_child_pid() -> u32 {
        #[cfg(unix)]
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut child = child;
        child.kill().unwrap();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn dead_holder_pid_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let dead_pid = reaped_child_pid();
        std::fs::write(lock_path(dir.path()), format!("{dead_pid}\n")).unwrap();
        let lock = acquire_active_state_lock(dir.path()).unwrap();
        let content = std::fs::read_to_string(&lock).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());
        release_active_state_lock(&lock);
        assert!(!lock_path(dir.path()).exists());
    }

    /// An empty lock is a writer between `create_new` and its pid write, or one
    /// that died in between. Both resolve by waiting briefly and then taking
    /// over — never by refusing for ten minutes while the ledger change that
    /// triggered the write sits already committed.
    #[test]
    fn an_abandoned_empty_lock_is_taken_over_after_the_grace_period() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), "").unwrap();
        let started = std::time::Instant::now();
        let lock = acquire_active_state_lock(dir.path()).unwrap();
        // The real constant, not a test override: the point is that the wait is
        // short in production.
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?} — the ten-minute refusal is back",
            started.elapsed()
        );
        let content = std::fs::read_to_string(&lock).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());
        release_active_state_lock(&lock);
        assert!(!lock_path(dir.path()).exists());
    }

    /// The mid-acquire window itself: a writer that has created the file but not
    /// yet written its pid must not cost the loser anything. With a zero grace
    /// period the takeover is immediate; the test asserts the acquire *succeeds*
    /// either way, which is what the field failure got wrong.
    #[test]
    fn a_fresh_empty_lock_does_not_block_the_next_writer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), "").unwrap();
        let lock = acquire_active_state_lock_with(dir.path(), Duration::ZERO).unwrap();
        assert_eq!(
            std::fs::read_to_string(&lock).unwrap().trim(),
            std::process::id().to_string()
        );
        release_active_state_lock(&lock);
    }

    /// Garbage content (not a pid, not empty) is classified like an empty lock:
    /// waited for, then taken over. It must never wedge the projection.
    #[test]
    fn an_unreadable_lock_is_taken_over_rather_than_wedging() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), "not-a-pid\n").unwrap();
        let lock = acquire_active_state_lock_with(dir.path(), Duration::ZERO).unwrap();
        assert_eq!(
            std::fs::read_to_string(&lock).unwrap().trim(),
            std::process::id().to_string()
        );
        release_active_state_lock(&lock);
    }

    /// The regression this fix exists for. `todo claim` from four workers at once
    /// failed on CI with "exists without a pid and is not stale … refusing
    /// takeover until it ages past 600s" — one loser landed in the window between
    /// the winner's create and its pid write. Threads reproduce the same window
    /// inside one process, so every acquirer must succeed.
    #[test]
    fn concurrent_acquires_all_succeed() {
        use std::sync::{Arc, Barrier};

        let rounds = 150;
        let workers = 4;
        for _ in 0..rounds {
            let dir = Arc::new(tempfile::tempdir().unwrap());
            let barrier = Arc::new(Barrier::new(workers));
            let mut handles = Vec::new();
            for _ in 0..workers {
                let dir = Arc::clone(&dir);
                let barrier = Arc::clone(&barrier);
                handles.push(std::thread::spawn(move || {
                    // Start together, so the create/write interleaving is real.
                    barrier.wait();
                    let lock = acquire_active_state_lock(dir.path()).unwrap();
                    let held = std::fs::read_to_string(&lock).unwrap();
                    assert_eq!(held.trim(), std::process::id().to_string());
                    release_active_state_lock(&lock);
                }));
            }
            for handle in handles {
                handle.join().unwrap();
            }
        }
    }

    /// A directory squatting on the lock name is not a lock: no pid can be
    /// read out of it and it cannot be unlinked as a file, so the acquire must
    /// fail with the removal error rather than spin or pretend to hold it.
    ///
    /// This is also the deterministic trigger for the two platform arms that
    /// guard the removal — on Windows `create_new` on a directory name reports
    /// `ERROR_ACCESS_DENIED` (not `AlreadyExists`), which is the same code a
    /// delete-pending lock reports, while on Unix it is `EEXIST`.
    #[test]
    fn a_directory_in_place_of_the_lock_reports_the_remove_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(lock_path(dir.path())).unwrap();
        let err = acquire_active_state_lock_with(dir.path(), Duration::ZERO).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("remove abandoned lock"),
            "unexpected error: {msg}"
        );
        // Nothing was acquired: the directory is still there.
        assert!(lock_path(dir.path()).is_dir());
    }

    /// A lock held by a pid that is gone is normally taken over by unlinking it.
    /// When the unlink itself is refused the error must name the removal, not be
    /// swallowed: the caller has to learn that the projection could not be
    /// written. Unix gets the refusal from a read-only parent directory.
    #[cfg(unix)]
    #[test]
    fn a_dead_holder_lock_that_cannot_be_unlinked_names_the_removal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), format!("{}\n", reaped_child_pid())).unwrap();
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(dir.path(), perms).unwrap();
        let err = acquire_active_state_lock_with(dir.path(), Duration::ZERO).unwrap_err();
        let msg = format!("{err:#}");
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(dir.path(), perms).unwrap();
        assert!(
            msg.contains("remove dead-holder lock"),
            "unexpected error: {msg}"
        );
    }

    /// Windows counterpart, and the deliberate counter-case to the delete-pending
    /// retry: a lock file another handle opened without delete sharing still
    /// reads fine (so it is classified by its dead pid) but cannot be unlinked.
    /// `DeleteFile` reports `ERROR_SHARING_VIOLATION`, which is *not* the
    /// delete-pending code, so the retry must not pretend it is transient — the
    /// acquire has to fail and name the removal.
    #[cfg(windows)]
    #[test]
    fn a_lock_held_closed_to_delete_names_the_removal() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, format!("{}\n", reaped_child_pid())).unwrap();
        // Readable by us, but nobody may delete it while this handle lives.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let err = acquire_active_state_lock_with(dir.path(), Duration::ZERO).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("remove dead-holder lock"),
            "unexpected error: {msg}"
        );
        // Still nothing acquired: the file we could not unlink is still the lock.
        drop(held);
        assert!(path.exists());
    }

    #[test]
    fn write_active_state_acquires_and_releases_lock() {
        let dir = tempfile::tempdir().unwrap();
        let goal = Goal::new("g", "lock objective", "/tmp");
        write_active_state(dir.path(), &goal).unwrap();
        assert!(dir.path().join("ACTIVE_GOAL_STATE.md").exists());
        // Released after the write — no residual lock file.
        assert!(!lock_path(dir.path()).exists());
    }

    /// The two ways `write_active_state` can fail before it publishes anything:
    /// a goal dir that cannot be created, and a lock a live writer holds. Either
    /// must surface as an error — publishing a projection from a lock-less write
    /// is exactly the corruption the sidecar exists to prevent.
    #[test]
    fn write_active_state_reports_an_unusable_goal_dir_and_a_held_lock() {
        let dir = tempfile::tempdir().unwrap();
        let goal = Goal::new("g", "lock objective", "/tmp");

        // A regular file where the goal dir should be: `create_dir_all` fails.
        let not_a_dir = dir.path().join("regular-file");
        std::fs::write(&not_a_dir, b"x").unwrap();
        let err = write_active_state(&not_a_dir, &goal).unwrap_err();
        assert!(
            !not_a_dir.join("ACTIVE_GOAL_STATE.md").exists(),
            "a failed acquire must not publish a projection: {err:#}"
        );

        // Our own pid is certainly alive → the lock is held, so the write waits
        // for the holder and then reports it instead of clobbering the file.
        std::fs::write(lock_path(dir.path()), format!("{}\n", std::process::id())).unwrap();
        let err = write_active_state(dir.path(), &goal).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("held by pid"), "unexpected error: {msg}");
        assert!(!dir.path().join("ACTIVE_GOAL_STATE.md").exists());
    }

    #[test]
    fn remove_lock_file_tolerates_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("never-created.lock");
        // NotFound is swallowed (a concurrent cleaner already removed it).
        assert!(remove_lock_file(&missing).is_ok());
    }

    #[test]
    fn remove_lock_file_surfaces_other_errors() {
        let dir = tempfile::tempdir().unwrap();
        // `remove_file` on a directory fails with EISDIR (not NotFound) on
        // unix — the "other error" arm surfaces it instead of swallowing.
        let subdir = dir.path().join("adir");
        std::fs::create_dir(&subdir).unwrap();
        assert!(remove_lock_file(&subdir).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn create_in_unwritable_dir_reports_contextual_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // goal_dir exists but is read-only → `create_new(true)` fails with a
        // non-AlreadyExists error (PermissionDenied) and surfaces the
        // "create ACTIVE_GOAL_STATE.md.lock" context.
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(dir.path(), perms).unwrap();
        let err = acquire_active_state_lock(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("create ACTIVE_GOAL_STATE.md.lock"),
            "unexpected error: {msg}"
        );
        // Restore writability so the tempdir can be cleaned up.
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(dir.path(), perms).unwrap();
    }
}
