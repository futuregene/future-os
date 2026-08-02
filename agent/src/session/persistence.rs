use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Weak,
    },
    time::Duration,
};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;

use super::{Manager, Session, SessionEntry, ENTRY_TYPE_SESSION_INFO};

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_QUEUE_CAPACITY: usize = 256;

enum PersistenceCommand {
    Append(Vec<SessionEntry>),
    UpdateInfo {
        key: String,
        value: serde_json::Value,
        ack: mpsc::SyncSender<std::result::Result<(), String>>,
    },
    RewriteRun {
        session: Session,
        ack: mpsc::SyncSender<std::result::Result<(), String>>,
    },
    /// Append the run's terminal entries (run_terminal marker + refreshed
    /// session_info) as an explicit durability boundary. Ordered after every
    /// mid-run append, so it observes any earlier append failure via last_error
    /// and refuses to commit an incomplete run.
    CommitRun {
        entries: Vec<SessionEntry>,
        ack: mpsc::SyncSender<std::result::Result<(), String>>,
    },
    Barrier(mpsc::SyncSender<std::result::Result<(), String>>),
}

struct WorkerSlot {
    generation: u64,
    sender: mpsc::SyncSender<PersistenceCommand>,
}

struct PersistenceInner {
    manager: Arc<Manager>,
    session_id: String,
    worker: Mutex<Option<WorkerSlot>>,
    next_generation: AtomicU64,
    closed: std::sync::atomic::AtomicBool,
    last_error: Mutex<Option<String>>,
    idle_timeout: Duration,
    #[cfg(test)]
    fail_next_rewrite: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_commit: std::sync::atomic::AtomicBool,
}

/// Ordered, lazily-started persistence queue for one session.
///
/// The worker exits after an idle period and is recreated on demand, so merely
/// hydrating historical sessions does not permanently allocate one thread per
/// session. All run-time appends, metadata updates, barriers, and final
/// rewrites for a live session share this ordering point.
#[derive(Clone)]
pub struct SessionPersistence {
    inner: Arc<PersistenceInner>,
}

impl SessionPersistence {
    pub fn new(manager: Arc<Manager>, session_id: String) -> Self {
        Self::with_idle_timeout(manager, session_id, DEFAULT_IDLE_TIMEOUT)
    }

    fn with_idle_timeout(
        manager: Arc<Manager>,
        session_id: String,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(PersistenceInner {
                manager,
                session_id,
                worker: Mutex::new(None),
                next_generation: AtomicU64::new(0),
                closed: std::sync::atomic::AtomicBool::new(false),
                last_error: Mutex::new(None),
                idle_timeout,
                #[cfg(test)]
                fail_next_rewrite: std::sync::atomic::AtomicBool::new(false),
                #[cfg(test)]
                fail_next_commit: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// Queue an append without blocking the model/tool future on filesystem
    /// locking or flush. The bounded queue reports overload explicitly instead
    /// of allowing an unhealthy disk to grow process memory without limit. A
    /// later rewrite/barrier observes accepted appends in FIFO order.
    pub fn append(&self, entries: Vec<SessionEntry>) -> Result<()> {
        self.try_send(PersistenceCommand::Append(entries))
    }

    /// Persist a session-info field in queue order.
    pub fn update_info(&self, key: &str, value: serde_json::Value) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.send_boundary(PersistenceCommand::UpdateInfo {
            key: key.to_string(),
            value,
            ack: ack_tx,
        })?;
        receive_ack(ack_rx)
    }

    /// Rewrite the completed in-memory run snapshot after every earlier append
    /// or metadata update. The worker merges the latest on-disk session-info
    /// fields so a config change made during the run cannot be overwritten by
    /// the run-start snapshot.
    pub fn rewrite_run_snapshot(&self, session: Session) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.send_boundary(PersistenceCommand::RewriteRun {
            session,
            ack: ack_tx,
        })?;
        receive_ack(ack_rx)
    }

    pub fn barrier(&self) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.send_boundary(PersistenceCommand::Barrier(ack_tx))?;
        receive_ack(ack_rx)
    }

    /// Append the run's terminal entries as this run's explicit durability
    /// boundary. Because the queue is FIFO, this runs after every mid-run
    /// append; a successful return means the whole run (user/assistant/tool +
    /// terminal marker + refreshed session_info) is durably on disk. Returns an
    /// error if any earlier append failed, signaling the caller to heal with a
    /// full rewrite instead of committing an incomplete run.
    pub fn commit_run(&self, entries: Vec<SessionEntry>) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.send_boundary(PersistenceCommand::CommitRun {
            entries,
            ack: ack_tx,
        })?;
        receive_ack(ack_rx)
    }

    /// Clear any recorded append error. Called at run start so the run-end
    /// commit decision (append-only commit vs healing rewrite) reflects only the
    /// current run's append health, not a stale error from an earlier run.
    pub fn reset_error(&self) {
        *self.inner.last_error.lock() = None;
    }

    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error.lock().clone()
    }

    /// Operator-triggered recovery boundary. Unlike CommitRun, this is allowed
    /// to supersede a prior writer error and records a conservative terminal
    /// outcome before the scheduler is released.
    pub fn recover_with_entries(&self, entries: Vec<SessionEntry>) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(anyhow!("session persistence is closed"));
        }
        self.inner
            .manager
            .append_entries_synced(&self.inner.session_id, &entries)?;
        *self.inner.last_error.lock() = None;
        Ok(())
    }

    /// Stop accepting persistence work and wait until every command accepted
    /// before this boundary has released its file handle. Deletion uses this
    /// before removing the transcript so a late writer cannot recreate it and
    /// Windows never has to unlink an open file. Prior write errors do not
    /// block close: deletion remains an allowed recovery operation while the
    /// session is persistence-degraded.
    pub fn close(&self) -> Result<()> {
        let worker = {
            let worker = self.inner.worker.lock();
            if self.inner.closed.swap(true, Ordering::AcqRel) {
                return Ok(());
            }
            worker
                .as_ref()
                .map(|slot| (slot.generation, slot.sender.clone()))
        };
        let Some((generation, sender)) = worker else {
            return Ok(());
        };
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        if sender.send(PersistenceCommand::Barrier(ack_tx)).is_err() {
            clear_worker_generation(&mut self.inner.worker.lock(), generation);
            return Ok(());
        }
        let _prior_result = ack_rx
            .recv()
            .map_err(|_| anyhow!("session persistence worker stopped before close boundary"))?;
        clear_worker_generation(&mut self.inner.worker.lock(), generation);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_rewrite(&self) {
        self.inner.fail_next_rewrite.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_commit(&self) {
        self.inner.fail_next_commit.store(true, Ordering::Release);
    }

    fn try_send(&self, mut command: PersistenceCommand) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(anyhow!("session persistence is closed"));
        }
        for _ in 0..2 {
            let mut worker = self.inner.worker.lock();
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(anyhow!("session persistence is closed"));
            }
            let (generation, sender) = self.ensure_worker_locked(&mut worker)?;
            match sender.try_send(command) {
                Ok(()) => return Ok(()),
                Err(mpsc::TrySendError::Full(_)) => {
                    let error = format!(
                        "session persistence queue is overloaded (capacity {COMMAND_QUEUE_CAPACITY})"
                    );
                    tracing::error!(session_id = %self.inner.session_id, "{error}");
                    *self.inner.last_error.lock() = Some(error.clone());
                    return Err(anyhow!(error));
                }
                Err(mpsc::TrySendError::Disconnected(returned)) => {
                    command = returned;
                    clear_worker_generation(&mut worker, generation);
                }
            }
        }
        let error = "session persistence worker is unavailable".to_string();
        *self.inner.last_error.lock() = Some(error.clone());
        Err(anyhow!(error))
    }

    /// Durability boundaries may wait behind already accepted appends. These
    /// calls run from blocking RPC/finalization contexts, never a Tokio worker.
    fn send_boundary(&self, mut command: PersistenceCommand) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(anyhow!("session persistence is closed"));
        }
        for _ in 0..2 {
            let mut worker = self.inner.worker.lock();
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(anyhow!("session persistence is closed"));
            }
            let (generation, sender) = self.ensure_worker_locked(&mut worker)?;
            match sender.send(command) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    command = error.0;
                    clear_worker_generation(&mut worker, generation);
                }
            }
        }
        Err(anyhow!("session persistence worker is unavailable"))
    }

    fn ensure_worker_locked(
        &self,
        worker: &mut Option<WorkerSlot>,
    ) -> Result<(u64, mpsc::SyncSender<PersistenceCommand>)> {
        if let Some(slot) = worker.as_ref() {
            return Ok((slot.generation, slot.sender.clone()));
        }
        let generation = self
            .inner
            .next_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let (sender, receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let weak = Arc::downgrade(&self.inner);
        let thread_sender = sender.clone();
        let name = format!("session-writer-{}", self.inner.session_id);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || run_worker(weak, generation, receiver))
            .map_err(|error| anyhow!("failed to spawn session persistence worker: {error}"))?;
        *worker = Some(WorkerSlot {
            generation,
            sender: thread_sender,
        });
        Ok((generation, sender))
    }
}

fn clear_worker_generation(worker: &mut Option<WorkerSlot>, generation: u64) {
    if worker
        .as_ref()
        .is_some_and(|slot| slot.generation == generation)
    {
        *worker = None;
    }
}

fn receive_ack(receiver: mpsc::Receiver<std::result::Result<(), String>>) -> Result<()> {
    receiver
        .recv()
        .map_err(|_| anyhow!("session persistence worker stopped before acknowledgement"))?
        .map_err(anyhow::Error::msg)
}

fn run_worker(
    inner: Weak<PersistenceInner>,
    generation: u64,
    receiver: mpsc::Receiver<PersistenceCommand>,
) {
    loop {
        let Some(state) = inner.upgrade() else {
            return;
        };
        let command = match receiver.recv_timeout(state.idle_timeout) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut worker = state.worker.lock();
                if worker
                    .as_ref()
                    .is_some_and(|slot| slot.generation == generation)
                {
                    // A sender may have cloned this generation's sender and
                    // queued a command after recv_timeout fired but before we
                    // acquired the worker lock. Drain that handoff while the
                    // lock excludes new senders; otherwise clearing the slot
                    // and dropping the receiver would lose an acknowledged
                    // append at the retirement boundary.
                    match receiver.try_recv() {
                        Ok(command) => {
                            drop(worker);
                            execute(&state, command);
                            continue;
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            *worker = None;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {}
                    }
                }
                return;
            }
        };
        execute(&state, command);
    }
}

fn execute(state: &PersistenceInner, command: PersistenceCommand) {
    match command {
        PersistenceCommand::Append(entries) => {
            let result = state
                .manager
                .append_entries(&state.session_id, &entries)
                .map_err(|error| error.to_string());
            record_result(state, &result, false);
        }
        PersistenceCommand::UpdateInfo { key, value, ack } => {
            let result = update_info(state, &key, value).map_err(|error| error.to_string());
            // A successful metadata update does not supersede a failed history
            // append, so keep any earlier error observable by barrier/finalize.
            record_result(state, &result, false);
            let _ = ack.send(result);
        }
        PersistenceCommand::RewriteRun { mut session, ack } => {
            merge_latest_session_info(state, &mut session);
            #[cfg(test)]
            let injected_failure = state.fail_next_rewrite.swap(false, Ordering::AcqRel);
            #[cfg(not(test))]
            let injected_failure = false;
            let result = if injected_failure {
                Err("injected session rewrite failure".to_string())
            } else {
                save_with_retry(&state.manager, &session).map_err(|error| error.to_string())
            };
            // A successful full snapshot contains the complete in-memory run,
            // so it is the one command that resolves earlier append failures.
            record_result(state, &result, true);
            let _ = ack.send(result);
        }
        PersistenceCommand::CommitRun { mut entries, ack } => {
            // A prior append failure means the on-disk run is incomplete and a
            // terminal marker cannot heal it. Refuse to commit so the caller
            // falls back to a full rewrite (which does heal). Otherwise append
            // the terminal entries with an fsync durability boundary.
            //
            // Bind the prior error to a local FIRST: matching directly on
            // `state.last_error.lock().clone()` would hold the mutex guard
            // across the whole match, deadlocking against record_result's
            // re-lock below (parking_lot is not re-entrant).
            let prior_error = state.last_error.lock().clone();
            let result = match prior_error {
                Some(error) => Err(format!(
                    "refusing to commit run: an earlier append failed ({error})"
                )),
                None => {
                    // The run's session_info was built from values frozen at run
                    // start; fold in any mid-run metadata change (rename / model
                    // / thinking / cwd / auto-compaction) that an update_info
                    // already persisted, so the commit's stale snapshot cannot
                    // revert it on disk. The rewrite path does the same merge.
                    if let Some(latest_info) = latest_session_info_content(state) {
                        if let Some(target_info) = entries
                            .iter_mut()
                            .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
                            .and_then(|entry| entry.content.as_mut())
                            .and_then(serde_json::Value::as_object_mut)
                        {
                            merge_session_info_keys(target_info, &latest_info);
                        }
                    }
                    #[cfg(test)]
                    let injected_failure = state.fail_next_commit.swap(false, Ordering::AcqRel);
                    #[cfg(not(test))]
                    let injected_failure = false;
                    let appended = if injected_failure {
                        Err("injected run commit failure".to_string())
                    } else {
                        state
                            .manager
                            .append_entries_synced(&state.session_id, &entries)
                            .map_err(|error| error.to_string())
                    };
                    record_result(state, &appended, false);
                    appended
                }
            };
            let _ = ack.send(result);
        }
        PersistenceCommand::Barrier(ack) => {
            let result = match state.last_error.lock().clone() {
                Some(error) => Err(error),
                None => Ok(()),
            };
            let _ = ack.send(result);
        }
    }
}

fn record_result(
    state: &PersistenceInner,
    result: &std::result::Result<(), String>,
    success_supersedes_prior: bool,
) {
    let mut last_error = state.last_error.lock();
    match result {
        Ok(()) if success_supersedes_prior => *last_error = None,
        Ok(()) => {}
        Err(error) => {
            tracing::error!(
                session_id = %state.session_id,
                "Session persistence command failed: {error}"
            );
            *last_error = Some(error.clone());
        }
    }
}

fn update_info(state: &PersistenceInner, key: &str, value: serde_json::Value) -> Result<()> {
    state
        .manager
        .update_session_info(&state.session_id, key, value)
}

/// Session-info keys that a mid-run `update_info` (rename / model / thinking /
/// cwd / auto-compaction) may change out-of-band. The append-only run commit
/// builds its `session_info` from values frozen at run *start*, so these keys
/// must be re-merged from the authoritative (last) on-disk snapshot or a mid-run
/// change is silently reverted on disk by the commit's stale snapshot.
const SESSION_INFO_MERGE_KEYS: &[&str] = &[
    "model",
    "thinking_level",
    "session_name",
    "cwd",
    "auto_compaction",
];

/// The authoritative (last) `session_info` content currently on disk, if any.
/// Single-writer FIFO ordering guarantees any mid-run `update_info` append that
/// preceded this command is already persisted, so this sees the freshest values.
fn latest_session_info_content(
    state: &PersistenceInner,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let latest = state.manager.load(&state.session_id).ok()?;
    latest
        .entries
        .iter()
        .rev()
        .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
        .and_then(|entry| entry.content.as_ref())
        .and_then(serde_json::Value::as_object)
        .cloned()
}

/// Copy the merge keys from `latest` over `target`, leaving every other field
/// (e.g. token counters, which the run commit refreshes) untouched.
fn merge_session_info_keys(
    target: &mut serde_json::Map<String, serde_json::Value>,
    latest: &serde_json::Map<String, serde_json::Value>,
) {
    for key in SESSION_INFO_MERGE_KEYS {
        if let Some(value) = latest.get(*key) {
            target.insert((*key).to_string(), value.clone());
        }
    }
}

fn merge_latest_session_info(state: &PersistenceInner, target: &mut Session) {
    let Some(latest_info) = latest_session_info_content(state) else {
        return;
    };
    let Some(target_info) = target
        .entries
        .iter_mut()
        .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
        .and_then(|entry| entry.content.as_mut())
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    merge_session_info_keys(target_info, &latest_info);
    target.model = target_info
        .get("model")
        .and_then(|value| value.as_str())
        .unwrap_or(&target.model)
        .to_string();
    target.name = target_info
        .get("session_name")
        .and_then(|value| value.as_str())
        .unwrap_or(&target.name)
        .to_string();
    target.cwd = target_info
        .get("cwd")
        .and_then(|value| value.as_str())
        .unwrap_or(&target.cwd)
        .to_string();
}

fn save_with_retry(manager: &Manager, session: &Session) -> Result<()> {
    let mut last_error = match manager.save(session) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    tracing::error!("Failed to save session (will retry): {last_error:#}");
    for attempt in 1..=5 {
        let wait_ms = 200_u64 << attempt;
        std::thread::sleep(Duration::from_millis(wait_ms));
        match manager.save(session) {
            Ok(()) => {
                tracing::info!("Session save succeeded on retry {attempt}");
                return Ok(());
            }
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (std::path::PathBuf, Arc<Manager>, Session) {
        let dir = std::env::temp_dir().join(format!(
            "future-session-writer-{}",
            crate::utils::generate_id()
        ));
        let manager = Arc::new(Manager::new(dir.clone()));
        let info = SessionEntry::session_info(
            serde_json::json!({
                "cwd": "/old",
                "model": "old-model",
                "thinking_level": "low",
                "session_name": "old name",
                "auto_compaction": true,
            }),
            "old-model".to_string(),
            "low".to_string(),
        );
        let session = Session::snapshot(
            "session-1".to_string(),
            "/old".to_string(),
            "old-model".to_string(),
            "old name".to_string(),
            String::new(),
            vec![
                info,
                SessionEntry::new_user("user", serde_json::json!("hello")),
            ],
        );
        manager.save(&session).unwrap();
        (dir, manager, session)
    }

    #[test]
    fn update_and_run_rewrite_are_fifo_and_preserve_latest_metadata() {
        let (_dir, manager, stale_snapshot) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_secs(1),
        );
        persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("partial"),
                vec![],
            )])
            .unwrap();
        persistence
            .update_info("model", serde_json::json!("new-model"))
            .unwrap();

        // Simulate finalization using a run-start snapshot. The writer must not
        // let it roll the newer model selection back.
        persistence.rewrite_run_snapshot(stale_snapshot).unwrap();
        persistence.barrier().unwrap();

        let loaded = manager.load("session-1").unwrap();
        assert_eq!(loaded.model, "new-model");
        let info = loaded
            .entries
            .iter()
            .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|entry| entry.content.as_ref())
            .unwrap();
        assert_eq!(info["model"], "new-model");
    }

    #[test]
    fn idle_worker_retires_and_restarts_without_losing_order() {
        let (_dir, manager, _) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_millis(20),
        );
        persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("first"),
                vec![],
            )])
            .unwrap();
        persistence.barrier().unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(persistence.inner.worker.lock().is_none());

        persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("second"),
                vec![],
            )])
            .unwrap();
        persistence.barrier().unwrap();
        let loaded = manager.load("session-1").unwrap();
        let assistant_count = loaded
            .entries
            .iter()
            .filter(|entry| entry.entry_type == super::super::ENTRY_TYPE_ASSISTANT)
            .count();
        assert_eq!(assistant_count, 2);
    }

    #[test]
    fn close_drains_prior_writes_and_rejects_late_recreation() {
        let (_dir, manager, _) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_secs(1),
        );
        persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("before close"),
                vec![],
            )])
            .unwrap();

        persistence.close().unwrap();
        let loaded = manager.load("session-1").unwrap();
        assert!(loaded
            .entries
            .iter()
            .any(|entry| { entry.content.as_ref() == Some(&serde_json::json!("before close")) }));

        std::fs::remove_file(manager.session_path("session-1")).unwrap();
        assert!(persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("after close"),
                vec![],
            )])
            .is_err());
        assert!(!manager.session_path("session-1").exists());
    }

    #[test]
    fn metadata_success_does_not_hide_an_earlier_append_failure() {
        let (dir, manager, session) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_secs(1),
        );
        std::fs::remove_file(manager.session_path("session-1")).unwrap();

        persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("not persisted"),
                vec![],
            )])
            .unwrap();
        assert!(persistence.barrier().is_err());

        manager.save(&session).unwrap();
        persistence
            .update_info("session_name", serde_json::json!("renamed"))
            .unwrap();
        assert!(
            persistence.barrier().is_err(),
            "a metadata write cannot supersede a lost history append"
        );

        persistence.rewrite_run_snapshot(session).unwrap();
        persistence.barrier().unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn idle_retirement_never_acknowledges_a_lost_command() {
        let (dir, manager, _) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_millis(1),
        );

        for index in 0..100 {
            std::thread::sleep(Duration::from_millis(1));
            persistence
                .append(vec![SessionEntry::new_assistant(
                    serde_json::json!(format!("entry-{index}")),
                    vec![],
                )])
                .unwrap();
            persistence.barrier().unwrap();
        }

        let loaded = manager.load("session-1").unwrap();
        let assistant_count = loaded
            .entries
            .iter()
            .filter(|entry| entry.entry_type == super::super::ENTRY_TYPE_ASSISTANT)
            .count();
        assert_eq!(assistant_count, 100);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn commit_run_appends_terminal_marker_durably() {
        let (_dir, manager, _) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_secs(1),
        );
        let terminal = SessionEntry::run_terminal(
            "run-commit",
            super::super::RUN_STATE_COMPLETED,
            9,
            100,
            None,
        );
        persistence.commit_run(vec![terminal]).unwrap();

        let loaded = manager.load("session-1").unwrap();
        let terminal_entry = loaded
            .entries
            .iter()
            .find(|e| e.entry_type == super::super::ENTRY_TYPE_RUN_TERMINAL)
            .expect("terminal marker must be persisted");
        assert_eq!(
            terminal_entry.content.as_ref().unwrap()["run_id"],
            "run-commit"
        );
    }

    #[test]
    fn commit_run_refuses_after_an_earlier_append_failure() {
        let (dir, manager, session) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_secs(1),
        );
        // Force the next append to fail by removing the file; the queued append
        // records the error, which barrier surfaces.
        std::fs::remove_file(manager.session_path("session-1")).unwrap();
        persistence
            .append(vec![SessionEntry::new_assistant(
                serde_json::json!("lost"),
                vec![],
            )])
            .unwrap();
        assert!(persistence.barrier().is_err());

        // Recreate the file: even though a write would now succeed, commit_run
        // must still refuse because an earlier append in this run was lost (the
        // on-disk run is incomplete; the caller heals via a full rewrite).
        manager.save(&session).unwrap();
        let terminal = SessionEntry::run_terminal(
            "run-commit",
            super::super::RUN_STATE_COMPLETED,
            9,
            100,
            None,
        );
        assert!(
            persistence.commit_run(vec![terminal]).is_err(),
            "commit must refuse when an earlier append failed"
        );
        // No terminal marker was written by the refused commit.
        let loaded = manager.load("session-1").unwrap();
        assert!(!loaded
            .entries
            .iter()
            .any(|e| e.entry_type == super::super::ENTRY_TYPE_RUN_TERMINAL));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn commit_run_preserves_mid_run_metadata_over_stale_snapshot() {
        let (dir, manager, _) = fixture();
        let persistence = SessionPersistence::with_idle_timeout(
            manager.clone(),
            "session-1".to_string(),
            Duration::from_secs(1),
        );
        // Mid-run the user changes model + name; update_info persists the new
        // values ahead of the run commit (single-writer FIFO ordering).
        persistence
            .update_info("model", serde_json::json!("mid-run-model"))
            .unwrap();
        persistence
            .update_info("session_name", serde_json::json!("Renamed Mid-Run"))
            .unwrap();

        // The run commit carries a session_info frozen at run START (stale model
        // + name) alongside fresh token fields, plus the terminal marker.
        let stale_info = SessionEntry::session_info(
            serde_json::json!({
                "cwd": "/old",
                "model": "old-model",
                "thinking_level": "low",
                "session_name": "old name",
                "auto_compaction": true,
                "tokens_in": 999,
                "tokens_out": 888,
            }),
            "old-model".to_string(),
            "low".to_string(),
        );
        let terminal =
            SessionEntry::run_terminal("run-merge", super::super::RUN_STATE_COMPLETED, 7, 50, None);
        persistence.commit_run(vec![stale_info, terminal]).unwrap();

        let loaded = manager.load("session-1").unwrap();
        // The authoritative (last) session_info must carry the mid-run values,
        // not the stale run-start snapshot — while keeping the commit's own
        // token fields (the merge only touches the metadata keys).
        let last_info = loaded
            .entries
            .iter()
            .rev()
            .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.as_ref())
            .expect("a session_info entry");
        assert_eq!(last_info["model"], "mid-run-model");
        assert_eq!(last_info["session_name"], "Renamed Mid-Run");
        assert_eq!(
            last_info["tokens_out"], 888,
            "token fields from the commit must survive the merge"
        );
        // The terminal marker remains the final durable record.
        assert_eq!(
            loaded.entries.last().unwrap().entry_type,
            super::super::ENTRY_TYPE_RUN_TERMINAL
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
