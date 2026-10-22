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
    let lock = lock_file(store, goal, "notes.lock")?;
    // A *blocking* lock, so contention waits instead of failing: an error here is
    // the file system refusing to lock at all, and a note that never reaches the
    // ledger because of that is worse than one whose deduplication raced — the
    // check below still runs, so a duplicate needs two writers in the same window.
    if let Err(error) = lock.lock_exclusive() {
        announce_lockless(&store.root_path(), &error);
    }
    if !store
        .events(goal)?
        .iter()
        .any(|e| matches!(&e.event, Event::SupervisorNote { dedup_key, .. } if dedup_key == key))
    {
        store.append(Event::SupervisorNote {
            goal_id: goal.into(),
            todo_id: todo.into(),
            note_kind: kind.into(),
            message: message.into(),
            dedup_key: key.into(),
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0200 | 0x0000_0008);
    }
    let log = lock_file(store, goal, "watch.log")?;
    cmd.stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = cmd.spawn()?;
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
    #[cfg(unix)]
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
