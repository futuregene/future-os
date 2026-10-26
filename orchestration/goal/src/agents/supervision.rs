//! Goal-plane-owned, non-LLM watchdog and durable supervisor outbox. The watchdog
//! only observes and delivers; retry/replan/model selection remain agent policy.
use crate::{
    agent_client::AgentClient,
    state::now_epoch,
    store::{Event, Store},
};
use anyhow::Result;
use fs2::FileExt;
use std::{collections::HashSet, fs::File, time::Duration};

const TICK: Duration = Duration::from_secs(2);
const MAX_BATCH_NOTES: usize = 32;

fn lock_file(store: &Store, goal: &str, suffix: &str) -> Result<File> {
    // IDs are not paths (also safe for legacy/user-selected goal IDs).
    use sha2::{Digest, Sha256};
    let name = format!("{:x}.{suffix}", Sha256::digest(goal.as_bytes()));
    let dir = std::path::PathBuf::from(store.root_path()).join("supervision");
    std::fs::create_dir_all(&dir)?;
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(name))?)
}

pub fn running(store: &Store, goal: &str) -> bool {
    // Two answers mean "do not assume the watcher's job is unowned". `Held`: another
    // process owns the loop. `Unknown`: the file system will not lock, so a watcher
    // may well be delivering and this probe must not claim otherwise. A lock that
    // cannot even be opened is different — no watcher can hold it then, and saying
    // "not running" is exactly true, which lets the caller deliver the note inline
    // (see `supervision_fault_points` for that contract).
    //
    // The spawn decision in `ensure_watchdog` is the one place those readings must
    // not be folded together: there, `Unknown` must still produce a watcher.
    matches!(watcher(store, goal), Ok(Watcher::Held | Watcher::Unknown))
}

/// What the watcher lock says about who is watching this goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Watcher {
    /// Nobody holds it: this process took it and let go again.
    Absent,
    /// Another process holds it, so it owns the watch loop.
    Held,
    /// The file opened but the file system will not lock it, so nothing can be
    /// told apart.
    ///
    /// `ENOLCK` on a network share without a lock manager, `ENOTSUP` on some FUSE
    /// mounts: `try_lock_exclusive` fails exactly as it does under contention, and
    /// calling both "somebody is watching" is what leaves a goal with **no**
    /// watcher at all — `watch` and `ensure_watchdog` both read that answer as
    /// "already covered" and stand down, and the supervisor's notes are then never
    /// delivered. The callers must treat this answer as a degraded sufficiency:
    /// run without mutual exclusion, and say so.
    Unknown,
}

/// Which watcher owns this goal, or why the question cannot be asked at all.
///
/// An `Err` is the lock file not being openable (its directory replaced by a file,
/// a read-only root): a real fault, distinct from a file system that opens the lock
/// and refuses to lock it, because only the second one can still be worked around.
fn watcher(store: &Store, goal: &str) -> Result<Watcher> {
    let file = lock_file(store, goal, "watch.lock")?;
    Ok(match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            Watcher::Absent
        }
        Err(error) if lock_is_contended(&error) => Watcher::Held,
        Err(_) => Watcher::Unknown,
    })
}

/// Whether a failed `try_lock_exclusive` means somebody else holds the lock.
///
/// `fs2` reports every failure the same way, and only one of them is contention;
/// `host/src/instance.rs` draws the same line for the Desktop's instance lock.
fn lock_is_contended(error: &std::io::Error) -> bool {
    error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

/// Say once — and only once per process — that the supervision locks cannot be
/// taken on this file system.
///
/// The three lock files guard three different things, but they fail for one
/// reason and have one fix, and the reader needs the sentence, not a stream of it:
/// the watcher ticks every two seconds and a note write happens on most ticks, so
/// an unguarded `eprintln!` would be a message every tick. Running degraded is a
/// deliberate choice (a goal with no watcher delivers nothing at all), so it is
/// said plainly rather than silently.
fn announce_lockless(root: &str, error: &std::io::Error) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "supervisor: {root} does not support file locks ({error}); supervisor notes still \
             flow, but two watchers — or two note writers — would no longer be held apart"
        );
    });
}

/// Persist even without a reachable agent or registered supervisor. Retry of
/// the same episode is a no-op, not another ledger row / paid wakeup.
pub fn queue(
    store: &mut Store,
    goal: &str,
    kind: &str,
    todo: &str,
    message: &str,
    key: &str,
) -> Result<()> {
    queue_many(
        store,
        goal,
        vec![Note {
            kind: kind.to_string(),
            todo: todo.to_string(),
            message: message.to_string(),
            key: key.to_string(),
        }],
    )
}

/// One ledger note waiting to be enqueued: what happened (`kind`), on which
/// todo, the sentence the supervisor reads, and the idempotency key that makes
/// re-observing the same episode a no-op.
pub struct Note {
    pub kind: String,
    pub todo: String,
    pub message: String,
    pub key: String,
}

/// Enqueue a batch of notes under ONE notes lock and ONE ledger read.
///
/// The dedup scan is the expensive part (it replays the goal), so a sweep that
/// has several observations to report reads the ledger once instead of once per
/// note — a watcher tick must stay cheap enough to run every two seconds on a
/// goal with dozens of finished todos.
pub fn queue_many(store: &mut Store, goal: &str, notes: Vec<Note>) -> Result<()> {
    if notes.is_empty() {
        return Ok(());
    }
    let lock = lock_file(store, goal, "notes.lock")?;
    // A *blocking* lock, so contention waits instead of failing: an error here is
    // the file system refusing to lock at all, and a note that never reaches the
    // ledger because of that is worse than one whose deduplication raced — the
    // check below still runs, so a duplicate needs two writers in the same window.
    if let Err(error) = lock.lock_exclusive() {
        announce_lockless(&store.root_path(), &error);
    }
    let mut seen: HashSet<String> = store
        .events(goal)?
        .into_iter()
        .filter_map(|entry| match entry.event {
            Event::SupervisorNote { dedup_key, .. } => Some(dedup_key),
            _ => None,
        })
        .collect();
    for note in notes {
        if !seen.insert(note.key.clone()) {
            continue;
        }
        store.append(Event::SupervisorNote {
            goal_id: goal.into(),
            todo_id: note.todo,
            note_kind: note.kind,
            message: note.message,
            dedup_key: note.key,
            ts: now_epoch(),
        })?;
    }
    Ok(())
}

/// Prepare a fixed-size immutable batch or retry an unacknowledged one. A
/// crash after remote acceptance reuses its key AND payload on the next tick.
pub async fn flush(store: &mut Store, goal_id: &str, client: &mut AgentClient) -> Result<()> {
    let lock = lock_file(store, goal_id, "outbox.lock")?;
    match lock.try_lock_exclusive() {
        Ok(()) => {}
        // Another delivery pass is already preparing or retrying this batch.
        Err(error) if lock_is_contended(&error) => return Ok(()),
        // No lock support: one eventual delivery matters more than a possible
        // duplicate, and skipping here would drop the batch on every tick — with
        // the watcher and the inline path both declining, nobody would send it.
        Err(error) => announce_lockless(&store.root_path(), &error),
    }
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(());
    };
    let Some(session) = goal.supervisor_session_id else {
        return Ok(());
    };
    let events = store.events(goal_id)?;
    let delivered: HashSet<&str> = events
        .iter()
        .filter_map(|e| match &e.event {
            Event::SupervisorBatchDelivered { batch_id, .. } => Some(batch_id.as_str()),
            _ => None,
        })
        .collect();
    let delivered_keys: HashSet<&str> = events
        .iter()
        .flat_map(|entry| match &entry.event {
            Event::SupervisorBatchPrepared {
                batch_id,
                note_keys,
                ..
            } if delivered.contains(batch_id.as_str()) => {
                note_keys.iter().map(String::as_str).collect()
            }
            _ => Vec::new(),
        })
        .collect();
    let mut assigned = HashSet::new();
    let mut pending = None;
    for event in &events {
        if let Event::SupervisorBatchPrepared {
            batch_id,
            session_id,
            note_keys,
            message,
            ..
        } = &event.event
        {
            // A supervisor change re-targets undelivered work, not every
            // historical notification already accepted by an earlier session.
            if session_id == &session || delivered.contains(batch_id.as_str()) {
                assigned.extend(note_keys.iter().cloned());
            }
            if session_id == &session
                && pending.is_none()
                && !delivered.contains(batch_id.as_str())
                && !note_keys
                    .iter()
                    .all(|key| delivered_keys.contains(key.as_str()))
            {
                pending = Some((batch_id.clone(), message.clone()));
            }
        }
    }
    if pending.is_none() {
        let mut keys = Vec::new();
        let mut lines = Vec::new();
        for event in &events {
            if let Event::SupervisorNote {
                dedup_key, message, ..
            } = &event.event
            {
                if !assigned.contains(dedup_key) {
                    assigned.insert(dedup_key.clone());
                    keys.push(dedup_key.clone());
                    // Delivery receipts are already bounded and carry final
                    // outcome + evidence pointers; do not clip them at 800 chars.
                    lines.push(crate::executor::truncate_evidence(message, 4096));
                    if keys.len() == MAX_BATCH_NOTES {
                        break;
                    }
                }
            }
        }
        if keys.is_empty() {
            return Ok(());
        }
        let batch_id = uuid::Uuid::new_v4().to_string();
        let message = format!("[future-goal] goal {goal_id}: {} event(s). Reconcile current status before acting; events may be stale. Read `future goal supervisor events --goal {goal_id}` for full evidence.\n{}", keys.len(), lines.join("\n"));
        store.append(Event::SupervisorBatchPrepared {
            goal_id: goal_id.into(),
            batch_id: batch_id.clone(),
            session_id: session.clone(),
            note_keys: keys,
            message: message.clone(),
            ts: now_epoch(),
        })?;
        pending = Some((batch_id, message));
    }
    if let Some((id, message)) = pending {
        tokio::time::timeout(
            Duration::from_secs(10),
            // Coalescing, not plain enqueue: the orchestrator is the one session
            // here whose queue is a stream of state syncs. While it is busy with
            // a run, the batches arriving behind it fold into ONE next run
            // instead of each starting (and usually being superseded) on its own.
            client.prompt_coalescing(&session, &message, &format!("goal-batch:{id}")),
        )
        .await??;
        store.append(Event::SupervisorBatchDelivered {
            goal_id: goal_id.into(),
            batch_id: id,
            ts: now_epoch(),
        })?;
    }
    Ok(())
}

pub fn pending_delivery(store: &Store, goal_id: &str) -> Result<bool> {
    if store.replay(goal_id)?.is_none() {
        return Ok(false);
    }
    let events = store.events(goal_id)?;
    let receipts: HashSet<&str> = events
        .iter()
        .filter_map(|e| match &e.event {
            Event::SupervisorBatchDelivered { batch_id, .. } => Some(batch_id.as_str()),
            _ => None,
        })
        .collect();
    let delivered: HashSet<&str> = events
        .iter()
        .flat_map(|e| match &e.event {
            Event::SupervisorBatchPrepared {
                batch_id,
                note_keys,
                ..
            } if receipts.contains(batch_id.as_str()) => {
                note_keys.iter().map(String::as_str).collect()
            }
            _ => Vec::new(),
        })
        .collect();
    Ok(events.iter().any(|e| matches!(&e.event, Event::SupervisorNote { dedup_key, .. } if !delivered.contains(dedup_key.as_str()))))
}

/// Note kind for "this advancement work is finished and awaits acceptance".
///
/// The two other note kinds this module produces are both failures
/// (`host_died`, `verification_due`): a goal whose work simply landed and was
/// accepted on time produced **no** note at all, so a supervisor that is not
/// already polling learns nothing when a worker finishes cleanly. This kind is
/// the missing half — a report, not a verdict: it never records or implies
/// verified/failed, and it does not age (see `record_completions`).
pub const TODO_DONE_NOTE_KIND: &str = "todo_done";

/// The note kind the run loop already emits when a turn completes a todo
/// (`console::notify_supervisor(..., "completed", ...)`). `record_completions`
/// reads it so one completion is reported once, whichever path observed it.
const COMPLETION_REPORT_KIND: &str = "completed";

/// How far back `record_completions` will look for a completion nobody has
/// reported yet.
///
/// The note is normally enqueued by the command that completed the todo (and by
/// the watcher's next tick, two seconds later), so this window only covers a
/// completion whose process died between the ledger write and the note — a
/// restart catch-up, not a historical backfill. Without a bound, the first tick
/// after an upgrade would queue one note per advancement todo ever finished,
/// and the supervisor would be woken about work it accepted weeks ago.
const COMPLETION_NOTE_WINDOW_SECS: u64 = 900;

/// Queue the acceptance reminder for advancement todos that reached `done` and
/// have not been reported yet.
///
/// This is deliberately independent of the delivery/verification verdict: a
/// note says "this work is finished, look at it", and it stays in the ledger
/// when the supervisor later records `verified` — nothing here invalidates it,
/// and the dedup key (`todo id` + `completed_at`) keeps the next sweep from
/// repeating it.
pub fn record_completions(store: &mut Store, goal_id: &str) -> Result<()> {
    use crate::state::{TaskClass, TodoStatus};
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(());
    };
    let now = now_epoch();
    let finished: Vec<(String, u64)> = goal
        .todos
        .iter()
        .filter(|todo| todo.class == TaskClass::Advancement && todo.status == TodoStatus::Done)
        .filter_map(|todo| {
            let completed_at = todo.completed_at?;
            if now.saturating_sub(completed_at) > COMPLETION_NOTE_WINDOW_SECS {
                return None;
            }
            Some((todo.id.clone(), completed_at))
        })
        .collect();
    if finished.is_empty() {
        return Ok(());
    }
    // One completion is one report. The run loop already reports the turn that
    // completed the todo (`completed`, carrying the evidence pointer), so a
    // second note for that same completion would wake the supervisor about an
    // event it has just been told about. A report from BEFORE this completion
    // does not count — a rework cycle that completes again gets its own note,
    // because the completion timestamp it is keyed on is newer.
    let already_reported: HashSet<(String, u64)> = store
        .events(goal_id)?
        .into_iter()
        .filter_map(|entry| match entry.event {
            Event::SupervisorNote {
                todo_id,
                note_kind,
                ts,
                ..
            } if note_kind == COMPLETION_REPORT_KIND => Some((todo_id, ts)),
            _ => None,
        })
        .collect();
    let notes: Vec<Note> = finished
        .into_iter()
        .filter(|(todo_id, completed_at)| {
            !already_reported
                .iter()
                .any(|(reported, ts)| reported == todo_id && ts >= completed_at)
        })
        .map(|(todo_id, completed_at)| Note {
            kind: TODO_DONE_NOTE_KIND.to_string(),
            message: format!(
                "[future-goal] goal {goal_id}: todo {todo_id} completed — the work landed and \
                 awaits acceptance; record verified/failed/rework. This note reports the \
                 completion, it is not a verdict."
            ),
            key: format!("todo_done:{todo_id}:{completed_at}"),
            todo: todo_id,
        })
        .collect();
    queue_many(store, goal_id, notes)
}

/// One observation pass over a goal: dead holders, overdue verifications, and
/// finished work nobody has reported. Every step is idempotent, which is what
/// lets the watcher tick and the cheap command paths run it unconditionally.
pub fn sweep(store: &mut Store, goal_id: &str) -> Result<()> {
    record_dead_holders(store, goal_id)?;
    record_completions(store, goal_id)
}

/// Re-arm supervision on a path an operator or orchestrator already runs.
///
/// The point is the failure mode this closes: the watcher is a detached process
/// with two spawn sites, and when it dies (host restart, a crash, an operator
/// killing it) nothing ever started another one — the goal then kept accepting
/// work while its notes sat in the outbox forever.
///
/// Cheap by construction, because this runs on ordinary commands:
///
/// * the watcher lock is probed **first** — `Held` returns immediately, with no
///   ledger replay, no child process and no added latency (that is the normal
///   case, and `status` polling it repeatedly must stay free);
/// * a goal with no registered supervisor is left alone: nothing to watch, and
///   nothing to deliver to. Its notes are still recorded by the watcher of any
///   goal that has one and by `scheduler tick`;
/// * otherwise it replays the goal, runs the observation `sweep` nobody else is
///   running, and spawns a watcher;
/// * a terminal goal with a drained outbox has nothing left to watch, so it is
///   left alone — but a terminal goal with a pending delivery is delivered
///   inline first (that is the last note of a finished goal).
///
/// Failures are split deliberately: a note the ledger refuses to take is
/// returned to the caller (a real fault — the operator is owed the message),
/// while a watcher that could not be spawned is reported and dropped, because a
/// command must never fail over its sidecar.
pub async fn rearm(store: &mut Store, goal_id: &str) -> Result<()> {
    match watcher(store, goal_id) {
        // Somebody owns the loop; its next tick (2 s) reports what is pending.
        Ok(Watcher::Held) => return Ok(()),
        Ok(Watcher::Absent | Watcher::Unknown) => {}
        // No watcher can hold a lock that cannot be opened, so this is still a
        // reason to re-arm and deliver — said below rather than swallowed here.
        Err(error) => eprintln!("supervisor: cannot probe the watcher lock: {error}"),
    }
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(());
    };
    if goal.status == "cancelled" || goal.supervisor_session_id.is_none() {
        return Ok(());
    }
    let terminal = goal.is_terminal();
    if terminal && !pending_delivery(store, goal_id)? {
        return Ok(());
    }
    sweep(store, goal_id)?;
    if !terminal {
        if let Err(error) = ensure_watchdog(store, goal_id) {
            // Already reported where it happened (a job that refuses breakaway,
            // a binary that cannot be re-executed); the command proceeds.
            eprintln!("supervisor: no watcher for {goal_id}: {error:#}");
        }
    }
    // The inline delivery, and the reason "the watcher died" is not the same as
    // "never reported": an environment that cannot spawn a watcher (the
    // documented opt-out, an embedder whose `current_exe` is not the CLI) still
    // pushes what is waiting, exactly as `notify_supervisor` does.
    if !running(store, goal_id) && pending_delivery(store, goal_id)? {
        let connected = tokio::time::timeout(
            Duration::from_secs(5),
            AgentClient::connect(&crate::agent_client::agent_addr()),
        )
        .await;
        if let Ok(Ok(mut client)) = connected {
            if let Err(error) = flush(store, goal_id, &mut client).await {
                eprintln!("supervisor outbox retained for retry: {error}");
            }
        }
    }
    Ok(())
}

pub fn record_dead_holders(store: &mut Store, goal_id: &str) -> Result<()> {
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(());
    };
    for todo in crate::work_items::task_lease::dead_holder_todos(&goal) {
        if let Some(pid) = todo.holder_pid {
            queue(store, goal_id, "host_died", &todo.id,
                &format!("[future-goal] goal {goal_id}: todo {} stopped before completion (host_died) — holder pid {pid} is gone; inspect then relaunch if appropriate", todo.id),
                &format!("host_died:{}:{pid}:{}", todo.id, todo.lease_expires_at.unwrap_or(0)))?;
        }
    }
    // Wall-clock follow-up still works when no further paid turns happen.
    for delivery in &goal.delivery_states {
        if delivery.outcome == "delivered" && now_epoch().saturating_sub(delivery.updated_at) >= 300
        {
            queue(store, goal_id, "verification_due", &delivery.todo_id,
                &format!("Delivery {} is awaiting verification; read its artifacts and record verified/failed/rework, not another blind retry.", delivery.todo_id),
                &format!("verification_due:{}:{}", delivery.todo_id, delivery.seq))?;
        }
    }
    Ok(())
}

/// Foreground service entry point. Its lifetime is independent of every run;
/// an OS lock prevents duplicate watchers. Restart via this same CLI after a
/// host restart (no claim of surviving power loss without an OS service).
pub async fn watch(store: &mut Store, goal_id: &str, once: bool) -> Result<()> {
    let lock = lock_file(store, goal_id, "watch.lock")?;
    match lock.try_lock_exclusive() {
        Ok(()) => {}
        // Another watcher owns the loop.
        Err(error) if lock_is_contended(&error) => return Ok(()),
        // No lock support here: watching without mutual exclusion still delivers
        // the notes, and standing down would deliver none — the failure mode the
        // tree already has a name for ("a goal with no watcher and no report").
        Err(error) => announce_lockless(&store.root_path(), &error),
    }
    loop {
        let Some(goal) = store.replay(goal_id)? else {
            return Ok(());
        };
        if goal.status == "cancelled"
            || (goal.is_terminal()
                && (goal.supervisor_session_id.is_none() || !pending_delivery(store, goal_id)?))
        {
            return Ok(());
        }
        record_dead_holders(store, goal_id)?;
        // The completion half of the same pass: finshed advancement work is
        // reported here too, so a `todo complete` from a worker that then died
        // still reaches the supervisor.
        record_completions(store, goal_id)?;
        if goal.supervisor_session_id.is_some() && pending_delivery(store, goal_id)? {
            let connected = tokio::time::timeout(
                Duration::from_secs(5),
                AgentClient::connect(&crate::agent_client::agent_addr()),
            )
            .await;
            if let Ok(Ok(mut client)) = connected {
                if let Err(error) = flush(store, goal_id, &mut client).await {
                    eprintln!("supervisor outbox retained for retry: {error}");
                } else if goal.is_terminal() && !pending_delivery(store, goal_id)? {
                    return Ok(());
                }
            }
        }
        if once {
            return Ok(());
        }
        tokio::time::sleep(TICK).await;
    }
}

/// Dispatch a separate watcher from production binaries only. Test harnesses
/// and foreground embedders opt in with `supervisor watch` explicitly.
pub fn ensure_watchdog(store: &Store, goal: &str) -> Result<()> {
    // `Held` is the only answer that means "already covered" here: an unknown
    // lock must still produce a watcher, because the child will take the same
    // degraded path and deliver what nobody else would. A lock that cannot even be
    // opened is reported now, before a child is spawned to fail at the same thing.
    match watcher(store, goal) {
        Ok(Watcher::Held) => return Ok(()),
        Ok(Watcher::Absent | Watcher::Unknown) => {}
        Err(error) => {
            anyhow::bail!("supervisor watchdog cannot open its lock for goal {goal}: {error}")
        }
    }
    if std::env::var("FUTURE_GOAL_NO_DETACH").ok().as_deref() == Some("1") {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !matches!(stem, "future" | "future-goal") {
        return Ok(());
    }
    let mut cmd = std::process::Command::new(&exe);
    if stem == "future" {
        cmd.arg("goal");
    }
    cmd.args(["supervisor", "watch", "--goal", goal]);
    cmd.env("FUTURE_GOAL_ROOT", store.root_path());
    let log = lock_file(store, goal, "watch.log")?;
    cmd.stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = crate::detach::spawn(&mut cmd)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        // The child's own exit is checked first: it is the one signal that stays
        // truthful on a file system whose locks mean nothing (the `Unknown` case,
        // where the watcher check below is satisfied the moment anything exists).
        if let Some(status) = child.try_wait()? {
            let closed = store
                .replay(goal)?
                .is_none_or(|g| g.status == "cancelled" || g.is_terminal());
            if status.success() && closed {
                return Ok(());
            }
            anyhow::bail!("supervisor watchdog exited before acquiring its lock: {status}");
        }
        if !matches!(watcher(store, goal), Ok(Watcher::Absent)) {
            // `Held` is the child (or a concurrent spawn); `Unknown` is this file
            // system refusing to lock, where the child runs degraded rather than
            // at all. Neither is "wait longer".
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("supervisor watchdog did not become ready within 3 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Goal;

    /// The lock file `lock_file` derives for a goal, so a test can make it
    /// something that cannot be locked.
    #[cfg(target_os = "macos")]
    fn lock_path(store: &Store, goal: &str, suffix: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        std::path::PathBuf::from(store.root_path())
            .join("supervision")
            .join(format!("{:x}.{suffix}", Sha256::digest(goal.as_bytes())))
    }

    #[test]
    fn only_contention_counts_as_another_watcher() {
        assert!(lock_is_contended(&fs2::lock_contended_error()));
        for code in [libc::ENOLCK, libc::EOPNOTSUPP, libc::EACCES] {
            assert!(
                !lock_is_contended(&std::io::Error::from_raw_os_error(code)),
                "errno {code} is not another watcher"
            );
        }
    }

    /// A share that will not lock must not look like "a watcher is already
    /// running": every spawn decision reads that answer as "already covered", so
    /// the goal would end up with no watcher at all and its notes would never be
    /// delivered. It runs degraded instead — no duplicate guard, and a note still
    /// lands — which is what the dead-holder sweep below observes.
    ///
    /// A FIFO is how this test produces the failure: `open(O_RDWR)` on one
    /// succeeds while `flock` on it fails with `ENOTSUP` (probed; `/dev/null`
    /// *accepts* the lock). macOS-only for the same reason as the Desktop's own
    /// instance-lock test: the errno is a property of the platform.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_file_system_that_will_not_lock_still_gets_a_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
        let mut goal = Goal::new("g", "goal", ".");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "g".into(),
                ts: goal.created_at,
            })
            .unwrap();
        // A dead holder is what the watcher's first tick reports, so "did it run"
        // is observable as a ledger row.
        let mut todo = crate::state::Todo::advancement("t1", "work");
        todo.claimed_by = Some("alice".into());
        todo.holder_pid = Some(dead_pid());
        goal.todos.push(todo);
        store
            .append(Event::TodoAdded {
                goal_id: "g".into(),
                todo: goal.todos[0].clone(),
                ts: goal.created_at,
            })
            .unwrap();
        std::fs::create_dir_all(std::path::PathBuf::from(store.root_path()).join("supervision"))
            .unwrap();
        let path = lock_path(&store, "g", "watch.lock");
        let fifo = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo only reads the path built above; a failure is asserted.
        let created = unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) };
        let mkfifo_error = format!(
            "mkfifo {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
        assert_eq!(created, 0, "{mkfifo_error}");

        assert_eq!(
            watcher(&store, "g").expect("the lock file opens"),
            Watcher::Unknown,
            "the file system cannot tell, and that is its own answer"
        );
        // `running` stays conservative for its read-only callers…
        assert!(running(&store, "g"));
        // …while the watch loop, which is what actually delivers, still runs.
        watch(&mut store, "g", true).await.unwrap();
        assert!(
            store
                .events("g")
                .unwrap()
                .iter()
                .any(|e| matches!(&e.event, Event::SupervisorNote { .. })),
            "a degraded watcher must still report the dead holder"
        );
    }

    /// Spawn a child, kill it, reap it — its pid is now guaranteed dead.
    #[cfg(target_os = "macos")]
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .unwrap();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        pid
    }

    /// The batch writer is what keeps a sweep cheap: one ledger read for many
    /// observations, and the same dedup the single-note path had — a key already
    /// in the ledger is skipped, and a key repeated inside one batch is written
    /// once.
    #[test]
    fn a_batch_of_notes_dedupes_against_the_ledger_and_itself() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
        store.register(&Goal::new("g", "goal", ".")).unwrap();
        let note = |key: &str| Note {
            kind: TODO_DONE_NOTE_KIND.to_string(),
            todo: "t1".to_string(),
            message: format!("note {key}"),
            key: key.to_string(),
        };
        let count = |store: &Store| {
            store
                .events("g")
                .unwrap()
                .iter()
                .filter(|e| matches!(e.event, Event::SupervisorNote { .. }))
                .count()
        };

        queue_many(&mut store, "g", vec![note("k1"), note("k1"), note("k2")]).unwrap();
        assert_eq!(count(&store), 2, "the repeated key is written once");
        queue_many(&mut store, "g", vec![note("k1"), note("k3")]).unwrap();
        assert_eq!(count(&store), 3, "the ledger key k1 is not rewritten");
    }

    /// A completion inside the catch-up window is reported exactly once; an
    /// older one is not: the sweep is a restart net, not a history replay (an
    /// upgrade must not wake the supervisor about work accepted weeks ago).
    #[test]
    fn completions_are_reported_once_within_the_window_and_never_backfilled() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
        store.register(&Goal::new("g", "goal", ".")).unwrap();
        let now = now_epoch();
        for (id, ts) in [("recent", now), ("ancient", 1)] {
            store
                .append(Event::TodoAdded {
                    goal_id: "g".into(),
                    todo: crate::state::Todo::advancement(id, "work"),
                    ts,
                })
                .unwrap();
            store
                .append(Event::TodoCompleted {
                    goal_id: "g".into(),
                    todo_id: id.into(),
                    no_follow_up: true,
                    successor_ids: vec![],
                    evidence: Some("landed".into()),
                    ts,
                })
                .unwrap();
        }
        let keys = |store: &Store| -> Vec<String> {
            store
                .events("g")
                .unwrap()
                .into_iter()
                .filter_map(|e| match e.event {
                    Event::SupervisorNote { dedup_key, .. } => Some(dedup_key),
                    _ => None,
                })
                .collect()
        };

        record_completions(&mut store, "g").unwrap();
        assert_eq!(
            keys(&store),
            vec![format!("todo_done:recent:{now}")],
            "only the completion inside the window is reported"
        );
        // A second pass (the watcher's next tick) adds nothing.
        record_completions(&mut store, "g").unwrap();
        assert_eq!(keys(&store).len(), 1, "the completed todo is not re-noted");
        // And a goal that does not exist is a no-op, not an error.
        record_completions(&mut store, "goal_missing").unwrap();
    }

    /// One completion is one report: a completion the run loop already reported
    /// (`completed`, with its evidence) must not also produce an acceptance
    /// reminder for the same moment — that would wake the supervisor twice about
    /// one event. A *newer* completion after a rework cycle is a different
    /// moment and does get its own note.
    #[test]
    fn a_completion_the_run_loop_already_reported_is_not_reported_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
        store.register(&Goal::new("g", "goal", ".")).unwrap();
        let completed_at = now_epoch();
        store
            .append(Event::TodoAdded {
                goal_id: "g".into(),
                todo: crate::state::Todo::advancement("t1", "work"),
                ts: completed_at,
            })
            .unwrap();
        store
            .append(Event::TodoCompleted {
                goal_id: "g".into(),
                todo_id: "t1".into(),
                no_follow_up: true,
                successor_ids: vec![],
                evidence: Some("landed".into()),
                ts: completed_at,
            })
            .unwrap();
        store
            .append(Event::SupervisorNote {
                goal_id: "g".into(),
                todo_id: "t1".into(),
                note_kind: COMPLETION_REPORT_KIND.into(),
                message: "turn completed t1".into(),
                dedup_key: "completed:t1:run-1".into(),
                ts: completed_at,
            })
            .unwrap();

        record_completions(&mut store, "g").unwrap();
        let kinds: Vec<String> = store
            .events("g")
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.event {
                Event::SupervisorNote { note_kind, .. } => Some(note_kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            vec![COMPLETION_REPORT_KIND.to_string()],
            "the run loop's report already covers this completion: {kinds:?}"
        );
    }

    #[test]
    fn notes_persist_without_supervisor_and_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
        store.register(&Goal::new("g", "goal", ".")).unwrap();
        for _ in 0..3 {
            queue(&mut store, "g", "host_died", "t", "offline", "episode").unwrap();
        }
        assert_eq!(
            store
                .events("g")
                .unwrap()
                .iter()
                .filter(|e| matches!(e.event, Event::SupervisorNote { .. }))
                .count(),
            1
        );
        let lock = lock_file(&store, "g", "watch.lock").unwrap();
        lock.lock_exclusive().unwrap();
        assert!(running(&store, "g"));
        // Other parallel tests spawn subprocesses. A concurrent fork may briefly
        // inherit this file description, so drop alone need not release flock
        // before the next assertion. Explicitly release the test-owned lock.
        FileExt::unlock(&lock).unwrap();
        drop(lock);
        assert!(!running(&store, "g"));
    }
}
