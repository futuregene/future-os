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
    s.replace(' ', "%20").replace('+', "%2B")
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
// writes OUR pid into the lock file; on conflict we read the holder's pid
// and probe liveness (`kill -0`): a live holder is a hard error, a dead
// holder or an empty lock older than [`EMPTY_LOCK_STALE_AFTER`] is a zombie
// we clear and take over (O2: lock liveness self-heal).

/// How old an EMPTY (no pid) lock file must be before it counts as a zombie
/// and is taken over. A fresh empty lock is either a writer that has not
/// finished writing its pid yet, or one that crashed mid-acquire —
/// conservative: refuse takeover until it ages out.
pub const EMPTY_LOCK_STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Acquire the `ACTIVE_GOAL_STATE.md.lock` sidecar for `goal_dir`, writing
/// this process's pid into the file. On success the caller owns the lock and
/// MUST release it with [`release_active_state_lock`]; on failure a live
/// holder or a fresh empty lock is reported with a descriptive error.
pub fn acquire_active_state_lock(goal_dir: &Path) -> Result<PathBuf> {
    acquire_active_state_lock_with(goal_dir, EMPTY_LOCK_STALE_AFTER)
}

/// Testable variant: `empty_stale_after` overrides the empty-lock staleness
/// threshold.
fn acquire_active_state_lock_with(goal_dir: &Path, empty_stale_after: Duration) -> Result<PathBuf> {
    fs::create_dir_all(goal_dir)?;
    let lock_path = goal_dir.join("ACTIVE_GOAL_STATE.md.lock");
    // Bounded retry: takeover clears the file, but another process may win
    // the create race; re-check a few times.
    for _ in 0..4 {
        if lock_path.exists() {
            probe_and_takeover(&lock_path, empty_stale_after)?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                use std::io::Write;
                writeln!(f, "{}", std::process::id()).context("write pid into lock")?;
                return Ok(lock_path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).context("create ACTIVE_GOAL_STATE.md.lock"),
        }
    }
    bail!("could not acquire ACTIVE_GOAL_STATE.md.lock (contended)")
}

/// Check an existing lock file: a live pid holder is a hard error; a dead
/// holder or an empty lock past `empty_stale_after` is removed (zombie
/// takeover). A fresh empty lock is refused.
fn probe_and_takeover(lock_path: &Path, empty_stale_after: Duration) -> Result<()> {
    let raw = fs::read_to_string(lock_path).unwrap_or_default();
    match raw.trim().parse::<u32>() {
        Ok(pid) if pid_alive(pid) => {
            bail!("ACTIVE_GOAL_STATE.md.lock held by pid {pid}")
        }
        Ok(_) => remove_lock_file(lock_path).context("remove dead-holder lock"),
        Err(_) => {
            // Empty / garbage content: stale only past the age threshold.
            let mtime = fs::metadata(lock_path).and_then(|m| m.modified()).ok();
            let stale = mtime.is_some_and(|t| t.elapsed().is_ok_and(|el| el > empty_stale_after));
            if stale {
                remove_lock_file(lock_path).context("remove stale empty lock")
            } else {
                bail!(
                    "ACTIVE_GOAL_STATE.md.lock exists without a pid and is not stale (mtime {:?}); \
                     refusing takeover until it ages past {empty_stale_after:?}",
                    mtime
                )
            }
        }
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

/// Is the process with this pid still alive? Unix probes with `kill -0`
/// (no signal delivered). Non-unix platforms have no zero-cost probe here —
/// conservatively report alive so a lock is never stolen from a possibly
/// live holder (empty-lock aging still applies).
#[cfg(unix)]
pub(crate) fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 performs existence checking only; no signal is sent.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    // EPERM: the process exists but we may not signal it → alive.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub(crate) fn pid_alive(_pid: u32) -> bool {
    true
}

// ── <state>/runs/ (per-run history + index) ────────────────────────────────

/// Append one run's files (JSON + markdown + index row) into the goal's
/// state directory (`<cwd>/.future/loop/goals/<id>/runs/`).
pub fn write_run(goal_dir: &Path, goal_id: &str, record: &crate::state::RunRecord) -> Result<()> {
    use std::io::Write;
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
    let ts = format!("{}-{offset}", now.format("%Y-%m-%dT%H-%M-%S"));

    let json_payload = json!({
        "goal_id": goal_id,
        "timestamp": rfc3339(record.recorded_at),
        "turn": record.turn,
        "todo_id": record.todo_id,
        "run_id": record.run_id,
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

    // index.jsonl (append)
    let index_line = format!(
        "{}\n",
        json!({
            "goal_id": goal_id,
            "timestamp": rfc3339(record.recorded_at),
            "path": format!("goals/{goal_id}/runs/{ts}.json"),
            "turn": record.turn,
            "classification": record.terminal_state,
        })
    );
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("index.jsonl"))
        .and_then(|mut f| f.write_all(index_line.as_bytes()))
        .context("append runs/index.jsonl")?;
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
        line.push_str(&format!(" resume_when={rw}"));
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
        line.push_str(&format!(" note={note}"));
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

    #[test]
    fn dead_holder_pid_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        // Spawn a child and reap it — its pid is now guaranteed dead.
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .unwrap();
        let dead_pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        std::fs::write(lock_path(dir.path()), format!("{dead_pid}\n")).unwrap();
        let lock = acquire_active_state_lock(dir.path()).unwrap();
        let content = std::fs::read_to_string(&lock).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());
        release_active_state_lock(&lock);
        assert!(!lock_path(dir.path()).exists());
    }

    #[test]
    fn stale_empty_lock_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), "").unwrap();
        // Threshold zero: any empty lock counts as stale.
        let lock = acquire_active_state_lock_with(dir.path(), Duration::ZERO).unwrap();
        let content = std::fs::read_to_string(&lock).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());
        release_active_state_lock(&lock);
        assert!(!lock_path(dir.path()).exists());
    }

    #[test]
    fn fresh_empty_lock_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), "").unwrap();
        let err =
            acquire_active_state_lock_with(dir.path(), Duration::from_secs(3600)).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not stale"), "unexpected error: {msg}");
        assert!(lock_path(dir.path()).exists());
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
}
