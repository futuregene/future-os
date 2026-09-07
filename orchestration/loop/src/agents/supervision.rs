//! Loop-owned, non-LLM watchdog and durable supervisor outbox. The watchdog
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
    lock_file(store, goal, "watch.lock").is_ok_and(|file| file.try_lock_exclusive().is_err())
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
    lock.lock_exclusive()?;
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
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
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
        let message = format!("[future-loop] goal {goal_id}: {} event(s). Reconcile current status before acting; events may be stale. Read `future loop supervisor events --goal {goal_id}` for full evidence.\n{}", keys.len(), lines.join("\n"));
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
            client.prompt(&session, &message, &format!("loop-batch:{id}")),
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
                &format!("[future-loop] goal {goal_id}: todo {} stopped before completion (host_died) — holder pid {pid} is gone; inspect then relaunch if appropriate", todo.id),
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
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
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
    if running(store, goal) || std::env::var("FUTURE_LOOP_NO_DETACH").ok().as_deref() == Some("1") {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !matches!(stem, "future" | "future-loop") {
        return Ok(());
    }
    let mut cmd = std::process::Command::new(&exe);
    if stem == "future" {
        cmd.arg("loop");
    }
    cmd.args(["supervisor", "watch", "--goal", goal]);
    cmd.env("FUTURE_LOOP_ROOT", store.root_path());
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
        if running(store, goal) {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            let closed = store
                .replay(goal)?
                .is_none_or(|g| g.status == "cancelled" || g.is_terminal());
            if status.success() && closed {
                return Ok(());
            }
            anyhow::bail!("supervisor watchdog exited before acquiring its lock: {status}");
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
        drop(lock);
        assert!(!running(&store, "g"));
    }
}
